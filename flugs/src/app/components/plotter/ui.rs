use egui::Vec2;
use egui_plot::Legend;

use crate::app::components::{File, FileHandler, FileID};

impl super::Plotter {
    pub fn render(
        &mut self,
        file_handler: &mut FileHandler,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) {
        // Horizontal stripe of switch buttons enabeling/disabeling groups
        ui.horizontal(|ui| {
            for grp in file_handler.groups.iter_mut().filter_map(|x| x.as_mut()) {
                ui.toggle_value(&mut grp.is_plotted, &grp.name);
            }
        });

        // These are needed to apply modifications to the selected file.
        let mut spans = (0.0, 0.0);
        let mut drag = Vec2::default();

        let auto_bounds = self.mode == super::PlotterMode::Display;
        let allow_drag = self.selected_fid.is_none() && self.mode == super::PlotterMode::Display;

        self.files_plot_ids.drain();
        let response = egui_plot::Plot::new("Plot")
            .allow_drag(allow_drag)
            .auto_bounds(egui::Vec2b {
                x: auto_bounds,
                y: auto_bounds,
            })
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                // Plot integration region, if intgrate mode is active.
                if let super::PlotterMode::Integrate = self.mode {
                    if let Some((xmin, xmax)) = self.current_integral {
                        let y = plot_ui.plot_bounds().center().y;
                        plot_ui.line(
                            egui_plot::Line::new(vec![[xmin, y], [xmax, y]])
                                .color(egui::Color32::RED)
                                .width(3.0),
                        );
                    }
                    // Handle mouse clicks (draging integral area).
                    //
                    // Reading this before the if statement is required to avoid a dead lock.
                    let inside_plot = pointer_inside_plot(&plot_ui);
                    plot_ui.ctx().input(|i| {
                        if i.pointer.button_down(egui::PointerButton::Primary)
                            && plot_ui.response().contains_pointer()
                            && inside_plot
                        {
                            match (i.pointer.press_origin(), i.pointer.latest_pos()) {
                                (Some(origin), Some(current_position)) => {
                                    // Pointer positions are in screen coordinates and must be translated into
                                    // the coordinate system of the plot.
                                    let origin =
                                        plot_ui.transform().value_from_position(origin).x as f64;
                                    let current_position =
                                        plot_ui.transform().value_from_position(current_position).x
                                            as f64;
                                    // The values must be sorted ascendingly when stored in `current_integral`.
                                    let (origin, current_position) = (
                                        origin.min(current_position),
                                        origin.max(current_position),
                                    );
                                    self.current_integral = Some((origin, current_position))
                                }
                                _ => {}
                            }
                        }
                    });
                }

                for (_, grp) in file_handler
                    .groups
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(id, x)| Some(id).zip(x.as_mut()))
                {
                    if !grp.is_plotted {
                        continue;
                    }
                    for fid in grp.file_ids.iter() {
                        if let Some(file) = file_handler
                            .registry
                            .get(fid)
                            .filter(|file| file.get_cache().is_some())
                        {
                            let egui_id = self.plot(fid, file, &grp.name, plot_ui);
                            self.files_plot_ids.insert(egui_id, *fid);
                        }
                    }
                }
                drag = plot_ui.pointer_coordinate_drag_delta();
                spans = {
                    let bounds = plot_ui.plot_bounds();
                    let xspan = (bounds.max()[0] - bounds.min()[0]).abs();
                    let yspan = (bounds.max()[1] - bounds.min()[1]).abs();
                    (xspan, yspan)
                };
                self.current_plot_bounds = {
                    let [xmin, ymin] = plot_ui.plot_bounds().min();
                    let [xmax, ymax] = plot_ui.plot_bounds().max();
                    [xmin, xmax, ymin, ymax]
                };

                plot_ui.plot_bounds()
            });

        // Get modifier input (we need this here already, to disallow the plot
        // to be panned).
        let modifiers = ctx.input(|i| [i.modifiers.alt, i.modifiers.ctrl, i.modifiers.shift]);
        let modifier_down = modifiers.iter().any(|x| *x);
        let mouse_pressed = ctx.input(|i| i.pointer.primary_pressed());

        if let Some(hovered_fid) = response
            .hovered_plot_item
            .and_then(|id| self.files_plot_ids.get(&id))
        {
            // Select file, if its plot was clicked this frame.
            if mouse_pressed {
                self.selected_fid = Some(*hovered_fid);
            }
        } else {
            // If we clicked somewhere and no modifier was pressed, we deselect
            // the currently selected file.
            if mouse_pressed && !modifier_down {
                self.selected_fid = None;
            }
        }
        if let Some(selected_file) = self
            .selected_fid
            .and_then(|fid| file_handler.registry.get_mut(&fid))
        {
            // `yspan` is needed to determine speed of y-scaling.
            let yspan = response.inner.height();
            let should_modify = modifier_down && drag.length() > 0.0;
            if should_modify {
                self.manipulate_file(selected_file, modifiers, drag, yspan);
            }
        }
    }

    fn plot(
        &self,
        fid: &FileID,
        file: &File,
        group_name: &str,
        plot_iu: &mut egui_plot::PlotUi,
    ) -> egui::Id {
        if let Some(data) = file.get_cache() {
            let ymin = data
                .iter()
                .map(|[_, y]| y)
                .reduce(|current_min, yi| if yi < current_min { yi } else { current_min })
                .unwrap_or(&0.0);
            // Apply custom shifting/scaling to data.
            let data: Vec<[f64; 2]> = data
                .iter()
                .map(|[x, y]| {
                    [
                        x + file.properties.xoffset,
                        (y - ymin) * file.properties.yscale + file.properties.yoffset + ymin,
                    ]
                })
                .collect();

            // Plot the data.
            let color = auto_color(Into::<i32>::into(*fid));
            let width = if self.selected_fid.is_some_and(|sfid| sfid == *fid) {
                2.5
            } else {
                1.0
            };
            let name = if file.properties.alias.is_empty() {
                format!("{} ({})", file.file_name(), group_name)
            } else {
                format!("{} ({})", file.properties.alias, group_name)
            };
            let egui_id = name.clone().into();
            plot_iu.line(
                egui_plot::Line::new(data.to_owned())
                    .color(color)
                    .width(width)
                    .name(name)
                    .id(egui_id),
            );

            if self.mode == super::PlotterMode::Integrate {
                if let Some((xmin, xmax)) = self.current_integral {
                    plot_iu.line(
                        egui_plot::Line::new(
                            data.iter()
                                .filter_map(|[x, y]| {
                                    if x >= &xmin && x <= &xmax {
                                        Some([*x, *y])
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<[f64; 2]>>(),
                        )
                        .color(egui::Color32::from_rgba_premultiplied(255, 255, 255, 100))
                        .width(width)
                        .id(egui_id),
                    );
                }
            }

            egui_id
        } else {
            // This should never happen because if the filter applied in
            // `Plotter::render`.
            unreachable!(
                "unable to get cache for plotting for file '{}'",
                file.file_name()
            )
        }
    }

    pub fn integrate_menu(&mut self, file_handler: &mut FileHandler, ui: &mut egui::Ui) {
        ui.menu_button("Measure Integral", |ui| {
            if ui.button("Reset").clicked() {
                self.current_integral = None
            }
            if let super::PlotterMode::Integrate = self.mode {
                ui.checkbox(
                    &mut self.integrate_with_local_baseline,
                    "Use local baseline?",
                );
            }
            if let Some((a, b)) = self.current_integral.iter_mut().next() {
                ui.label(format!("Left bound: {a:.2e}"));
                ui.label(format!("Right bound: {b:.2e}"));

                for (_, grp) in file_handler
                    .groups
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(id, x)| Some(id).zip(x.as_mut()))
                {
                    if !grp.is_plotted {
                        continue;
                    }
                    for fid in grp.file_ids.iter() {
                        if let Some(file) = file_handler
                            .registry
                            .get(fid)
                            .filter(|file| file.get_cache().is_some())
                        {
                            ui.label(file.file_name());
                        }
                    }
                }
            } else {
                if ui.button("New Region").clicked() {
                    self.current_integral = Some((0.0, 0.0));
                }
            }
        });
    }
}

pub fn auto_color(color_idx: i32) -> egui::Color32 {
    // analog to egui_plot
    let golden_ratio = (5.0_f32.sqrt() - 1.0) / 2.0; // 0.61803398875
    let h = color_idx as f32 * golden_ratio;
    egui::epaint::Hsva::new(h, 0.85, 0.5, 1.0).into()
}

fn pointer_inside_plot(plot_ui: &egui_plot::PlotUi) -> bool {
    if let Some(pointer_position) = plot_ui.pointer_coordinate() {
        return plot_ui
            .plot_bounds()
            .range_x()
            .contains(&pointer_position.x)
            && plot_ui
                .plot_bounds()
                .range_y()
                .contains(&pointer_position.y);
    }
    return false;
}
