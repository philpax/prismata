use std::{
    fmt,
    sync::{Arc, LazyLock},
};

use std::collections::HashSet;

use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use bevy_egui::{egui, EguiPrimaryContextPass};
use egui::{Color32 as OldColor32, Vec2 as OldVec2};
use egui_aesthetix::Aesthetix;

use crate::{
    camera,
    color::{
        bevy_color_to_egui_color32, bevy_color_to_egui_hsv, egui_color32_to_bevy_color,
        egui_hsv_to_bevy_color,
    },
    load_save::{RequestLoad, RequestSave},
    raycast::{CursorRayHit, CursorRayHitWithoutDraft},
    rendering, tools, voxel, AppState, ProjectName,
};

#[derive(Resource, Deref, DerefMut)]
pub struct CursorVisible(pub bool);
/// Run condition
pub fn is_cursor_visible(cursor_visible: Res<CursorVisible>) -> bool {
    cursor_visible.0
}
/// Run condition
pub fn is_cursor_invisible(cursor_visible: Res<CursorVisible>) -> bool {
    !cursor_visible.0
}

#[derive(Resource, Deref, DerefMut)]
pub struct Toasts(egui_notify::Toasts);

#[derive(Resource)]
pub struct InspectorOpen(pub bool);

pub fn plugin(app: &mut App) {
    app.init_resource::<RightPanelDockState>()
        .insert_resource(CursorVisible(true))
        .insert_resource(Toasts(
            egui_notify::Toasts::default().with_anchor(egui_notify::Anchor::BottomRight),
        ))
        .insert_resource(InspectorOpen(false))
        .add_plugins(bevy_inspector_egui::DefaultInspectorConfigPlugin)
        .add_systems(Update, setup.run_if(run_once))
        .add_systems(
            PreUpdate,
            (
                absorb_egui_inputs
                    .after(bevy_egui::EguiPreUpdateSet::ProcessInput)
                    .before(bevy_egui::EguiPreUpdateSet::BeginPass),
                drop_egui_input_if_cursor_invisible
                    .after(bevy_egui::EguiPreUpdateSet::ProcessInput)
                    .before(bevy_egui::EguiPreUpdateSet::BeginPass),
            ),
        )
        .add_systems(
            EguiPrimaryContextPass,
            (
                (
                    cursor_grab,
                    reset_cursor_visibility_on_tool_change.run_if(tools::tool_just_changed),
                )
                    .chain(),
                (
                    ui_left_panel.run_if(in_state(AppState::Edit)),
                    ui_right_panel.run_if(in_state(AppState::Edit)),
                    ui_top_left_panel.run_if(in_state(AppState::Edit)),
                    ui_top_right_panel,
                    ui_toasts,
                    ui_inspector,
                ),
            ),
        );
}

// Returns a normalized scroll wheel value where each "click" is 1.0.
pub fn normalize_scroll_wheel(event: &MouseWheel) -> f32 {
    match event.unit {
        MouseScrollUnit::Line => event.y,
        // This is what it is for Chrome on my local system, unclear what
        // the actual standard is.
        MouseScrollUnit::Pixel => event.y / 120.0,
    }
}

fn setup(mut contexts: bevy_egui::EguiContexts, mut toasts: ResMut<Toasts>) {
    struct AesthetixWithNormalSpacing<A: egui_aesthetix::Aesthetix>(A);
    impl<A: egui_aesthetix::Aesthetix> egui_aesthetix::Aesthetix for AesthetixWithNormalSpacing<A> {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn primary_accent_color_visuals(&self) -> OldColor32 {
            self.0.primary_accent_color_visuals()
        }
        fn secondary_accent_color_visuals(&self) -> OldColor32 {
            self.0.secondary_accent_color_visuals()
        }
        fn bg_primary_color_visuals(&self) -> OldColor32 {
            self.0.bg_primary_color_visuals()
        }
        fn bg_secondary_color_visuals(&self) -> OldColor32 {
            self.0.bg_secondary_color_visuals()
        }
        fn bg_triage_color_visuals(&self) -> OldColor32 {
            self.0.bg_triage_color_visuals()
        }
        fn bg_auxiliary_color_visuals(&self) -> OldColor32 {
            self.0.bg_auxiliary_color_visuals()
        }
        fn bg_contrast_color_visuals(&self) -> OldColor32 {
            self.0.bg_contrast_color_visuals()
        }
        fn fg_primary_text_color_visuals(&self) -> Option<OldColor32> {
            self.0.fg_primary_text_color_visuals()
        }
        fn fg_success_text_color_visuals(&self) -> OldColor32 {
            self.0.fg_success_text_color_visuals()
        }
        fn fg_warn_text_color_visuals(&self) -> OldColor32 {
            self.0.fg_warn_text_color_visuals()
        }
        fn fg_error_text_color_visuals(&self) -> OldColor32 {
            self.0.fg_error_text_color_visuals()
        }
        fn dark_mode_visuals(&self) -> bool {
            self.0.dark_mode_visuals()
        }
        fn margin_style(&self) -> i8 {
            6
        }
        fn button_padding(&self) -> OldVec2 {
            OldVec2::new(6.0, 4.0)
        }
        fn item_spacing_style(&self) -> f32 {
            4.0
        }
        fn scroll_bar_width_style(&self) -> f32 {
            6.0
        }
        fn rounding_visuals(&self) -> u8 {
            4
        }
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let original_visuals = egui::Visuals::dark();
    let mut style = AesthetixWithNormalSpacing(egui_aesthetix::themes::NordDark).custom_style();
    style.visuals.popup_shadow = original_visuals.popup_shadow;
    style.visuals.window_shadow = original_visuals.window_shadow;
    ctx.set_style(Arc::new(style));

    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    toasts
        .info("Welcome! Select a tool and left-click to create.")
        .duration(Some(web_time::Duration::from_secs(5)));
}

fn ui_left_panel(mut contexts: bevy_egui::EguiContexts, active_tool: ResMut<tools::ActiveTool>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Area::new("Left".into())
        .anchor(egui::Align2::LEFT_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                tools::ui_left_panel(ui, active_tool);
            });
        });
}

#[derive(Resource)]
struct RightPanelDockState(egui_dock::DockState<RightPanelTab>);
impl Default for RightPanelDockState {
    fn default() -> Self {
        Self(egui_dock::DockState::new(RightPanelTab::ALL.to_vec()))
    }
}
fn ui_right_panel<'a>(
    mut right_panel_dock_state: ResMut<RightPanelDockState>,
    mut contexts: bevy_egui::EguiContexts,
    tool_color: ResMut<tools::ToolColor>,
    last_used_colors: Res<tools::LastUsedColors>,

    voxels_per_meter: ResMut<voxel::VoxelsPerMeter>,
    recreate_start_scene_writer: MessageWriter<voxel::RecreateStartScene>,
    project_name: ResMut<ProjectName>,
    request_save_writer: MessageWriter<RequestSave>,
    request_load_writer: MessageWriter<RequestLoad>,
    floor_color: ResMut<rendering::FloorColor>,
    sun_angle: ResMut<rendering::SunAngle>,

    chunk_visualization: ResMut<voxel::ChunkVisualization>,
    chunks: Option<Res<voxel::Chunks>>,
    chunk_datas: Query<(Entity, &'a voxel::ChunkCoords, &'a voxel::ChunkData)>,
    cursor_ray_hit: Res<CursorRayHit>,
    cursor_ray_hit_without_draft: Res<CursorRayHitWithoutDraft>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Area::new("Right".into())
        .anchor(egui::Align2::RIGHT_CENTER, egui::Vec2::ZERO)
        .default_size(egui::vec2(320.0, 670.0))
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                egui_dock::DockArea::new(&mut right_panel_dock_state.0).show_inside(
                    ui,
                    &mut RightPanelViewer {
                        tool_color,
                        last_used_colors,

                        voxels_per_meter,
                        recreate_start_scene_writer,
                        project_name,
                        request_save_writer,
                        request_load_writer,
                        floor_color,
                        sun_angle,

                        chunk_visualization,
                        chunks,
                        chunk_datas,
                        cursor_ray_hit,
                        cursor_ray_hit_without_draft,
                    },
                );
            });
        });
}
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum RightPanelTab {
    #[default]
    Tools,
    Settings,
    Info,
}
impl RightPanelTab {
    const ALL: &[Self] = &[Self::Tools, Self::Settings, Self::Info];
}
impl fmt::Display for RightPanelTab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
struct RightPanelViewer<'w, 's, 'a> {
    tool_color: ResMut<'w, tools::ToolColor>,
    last_used_colors: Res<'w, tools::LastUsedColors>,

    voxels_per_meter: ResMut<'w, voxel::VoxelsPerMeter>,
    recreate_start_scene_writer: MessageWriter<'w, voxel::RecreateStartScene>,
    project_name: ResMut<'w, ProjectName>,
    request_save_writer: MessageWriter<'w, RequestSave>,
    request_load_writer: MessageWriter<'w, RequestLoad>,
    floor_color: ResMut<'w, rendering::FloorColor>,
    sun_angle: ResMut<'w, rendering::SunAngle>,

    chunk_visualization: ResMut<'w, voxel::ChunkVisualization>,
    chunks: Option<Res<'w, voxel::Chunks>>,
    chunk_datas: Query<'w, 's, (Entity, &'a voxel::ChunkCoords, &'a voxel::ChunkData)>,
    cursor_ray_hit: Res<'w, CursorRayHit>,
    cursor_ray_hit_without_draft: Res<'w, CursorRayHitWithoutDraft>,
}
impl<'w, 's, 'a> egui_dock::TabViewer for RightPanelViewer<'w, 's, 'a> {
    type Tab = RightPanelTab;
    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.to_string().into()
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match *tab {
            RightPanelTab::Tools => self.tools(ui),
            RightPanelTab::Settings => self.settings(ui),
            RightPanelTab::Info => self.info(ui),
        }
    }
}
impl RightPanelViewer<'_, '_, '_> {
    fn tools(&mut self, ui: &mut egui::Ui) {
        static PALETTE: LazyLock<Vec<(Color, egui::Color32)>> = LazyLock::new(|| {
            include_str!("color_palette.txt")
                .lines()
                .filter(|l| !l.starts_with("//"))
                .map(|l| egui::Color32::from_hex(l).unwrap())
                .map(|c| (egui_color32_to_bevy_color(c), c))
                .collect()
        });

        ui.group(|ui| {
            ui.label("Tool Color");
            color_picker(ui, &mut self.tool_color.base);
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let spacing = ui.spacing_mut();
                spacing.item_spacing = egui::Vec2::ZERO;
                spacing.interact_size = egui::Vec2::ZERO;

                for color in self.last_used_colors.0 {
                    let egui_color = bevy_color_to_egui_color32(color);
                    color_palette_button(ui, &mut self.tool_color.base, color, egui_color);
                }
            });

            ui.add_space(8.0);
            ui.scope(|ui| {
                let spacing = ui.spacing_mut();
                spacing.item_spacing = egui::Vec2::ZERO;
                spacing.interact_size = egui::Vec2::ZERO;

                for row in PALETTE.chunks(16) {
                    ui.horizontal(|ui| {
                        for (color, egui_color) in row {
                            color_palette_button(
                                ui,
                                &mut self.tool_color.base,
                                *color,
                                *egui_color,
                            );
                        }
                    });
                }
            });

            ui.add(
                egui::Slider::new(&mut self.tool_color.color_noise, 0.0..=1.0).text("Color Noise"),
            );
            ui.add(
                egui::Slider::new(&mut self.tool_color.grayscale_noise, 0.0..=1.0)
                    .text("Grayscale Noise"),
            );
        });

        fn color_palette_button(
            ui: &mut egui::Ui,
            tool_color_base: &mut Color,
            color: Color,
            egui_color: egui::Color32,
        ) {
            const COLOR_SIZE: f32 = 12.;
            let (rect, response) =
                ui.allocate_at_least(egui::Vec2::splat(COLOR_SIZE), egui::Sense::click());
            let rounding = egui::CornerRadius::ZERO;
            let stroke = if response.hovered() {
                egui::Stroke::new(1.0, egui::Color32::WHITE)
            } else {
                egui::Stroke::NONE
            };
            ui.painter().rect(
                rect,
                rounding,
                egui_color,
                stroke,
                egui::StrokeKind::Outside,
            );
            if response.clicked() {
                *tool_color_base = color;
            }
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.request_save_writer.write(RequestSave);
                }
                if ui.button("Load").clicked() {
                    self.request_load_writer.write(RequestLoad);
                }
            });

            ui.group(|ui| {
                ui.label(egui::RichText::new("Project Name").underline());
                ui.text_edit_singleline(&mut self.project_name.0);
            });

            ui.group(|ui| {
                ui.label(egui::RichText::new("Voxels per meter").underline());
                ui.label("Changing this will clear your world!!!");
                let mut new_vpm = self.voxels_per_meter.0;
                ui.add(egui::Slider::new(&mut new_vpm, 1..=100));
                if new_vpm != self.voxels_per_meter.0 {
                    self.voxels_per_meter.0 = new_vpm;
                }
                if ui.button("Recreate Start Scene").clicked() {
                    self.recreate_start_scene_writer
                        .write(voxel::RecreateStartScene);
                }
            });

            ui.group(|ui| {
                ui.label(egui::RichText::new("Sun").underline());
                let mut sun_angle_deg = self.sun_angle.0.to_degrees();
                ui.add(egui::Slider::new(&mut sun_angle_deg, 0.0..=360.0).suffix("°"));
                self.sun_angle.0 = sun_angle_deg.to_radians();
            });

            ui.group(|ui| {
                ui.label(egui::RichText::new("Floor Color").underline());
                color_picker(ui, &mut self.floor_color.0);
            });
        });
    }
    fn info(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(format!(
                "Prismata v{}-{}",
                env!("CARGO_PKG_VERSION"),
                git_version::git_version!(fallback = "unknown")
            ))
            .underline(),
        );

        ui.label(format!(
            "Chunk count: {}",
            self.chunks
                .as_ref()
                .map(|c| c.len().to_string())
                .unwrap_or_else(|| "N/A".to_string())
        ));
        let actual_chunk_count = self.chunk_datas.iter().count();
        ui.label(format!("Chunk count (actual): {actual_chunk_count}"));

        let resource_chunk_ids = self
            .chunks
            .as_ref()
            .map(|c| c.0.iter().map(|p| (*p.0, *p.1)).collect::<HashSet<_>>())
            .unwrap_or_default();

        let actual_chunk_ids = self
            .chunk_datas
            .iter()
            .map(|cd| (*cd.1, cd.0))
            .collect::<HashSet<_>>();

        let shared_ids = resource_chunk_ids.intersection(&actual_chunk_ids).count();
        ui.label(format!(
            "Shared chunk IDs: {} ({:2}%)",
            shared_ids,
            shared_ids as f32 / actual_chunk_count as f32 * 100.0
        ));
        ui.checkbox(&mut self.chunk_visualization.0, "Show chunks");

        ui.label(egui::RichText::new("Cursor Ray Hit").underline());
        ui.label(format!("{:.2?}", self.cursor_ray_hit.0));

        ui.label(egui::RichText::new("Cursor Ray Hit Without Draft").underline());
        ui.label(format!("{:.2?}", self.cursor_ray_hit_without_draft.0));
    }
}

fn ui_top_left_panel(
    world: &mut World,
    egui_context_query: &mut QueryState<
        &'static mut bevy_egui::EguiContext,
        With<bevy_egui::PrimaryEguiContext>,
    >,
) {
    let Some(mut egui_context) = egui_context_query
        .single_mut(world)
        .ok()
        .map(|ctx| ctx.into_inner().clone())
    else {
        return;
    };
    egui::Area::new("TopLeft".into())
        .anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
        .show(egui_context.get_mut(), |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                tools::Tool::ui_top_left_panel(ui, world);
            });
        });
}

fn ui_top_right_panel(
    mut contexts: bevy_egui::EguiContexts,
    main_camera: Query<(&camera::CameraType, &camera::CameraController), With<camera::MainCamera>>,
    state: Res<State<AppState>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Area::new("TopRight".into())
        .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                let state = state.get().to_string();
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new("State:").underline()));
                    ui.label(state);
                    ui.label("(F5 to swap)");
                });
                camera::ui_top_right_panel(ui, main_camera);
            });
        });
}

fn ui_toasts(mut contexts: bevy_egui::EguiContexts, mut toasts: ResMut<Toasts>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    toasts.show(ctx);
}

fn ui_inspector(
    world: &mut World,
    egui_context_query: &mut QueryState<
        &'static mut bevy_egui::EguiContext,
        With<bevy_egui::PrimaryEguiContext>,
    >,
) {
    let Some(mut egui_context) = egui_context_query
        .single_mut(world)
        .ok()
        .map(|ctx| ctx.into_inner().clone())
    else {
        return;
    };

    let mut inspector_open = world.resource_mut::<InspectorOpen>().0;
    if world
        .resource::<ButtonInput<KeyCode>>()
        .just_pressed(KeyCode::F1)
    {
        inspector_open = !inspector_open;
    }
    egui::Window::new("Inspector")
        .open(&mut inspector_open)
        .show(egui_context.get_mut(), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // equivalent to `WorldInspectorPlugin`
                bevy_inspector_egui::bevy_inspector::ui_for_world(world, ui);

                egui::CollapsingHeader::new("Materials").show(ui, |ui| {
                    bevy_inspector_egui::bevy_inspector::ui_for_assets::<StandardMaterial>(
                        world, ui,
                    );
                });

                ui.heading("Entities");
                bevy_inspector_egui::bevy_inspector::ui_for_entities(world, ui);
            });
        });
    *world.resource_mut::<InspectorOpen>() = InspectorOpen(inspector_open);
}

// <https://github.com/mvlabat/bevy_egui/issues/47#issuecomment-1922695612>
fn absorb_egui_inputs(
    mut contexts: bevy_egui::EguiContexts,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut mouse_wheel: ResMut<Messages<MouseWheel>>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    if !(ctx.wants_pointer_input() || ctx.is_pointer_over_area()) {
        return;
    }
    let modifiers = [
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
    ];

    let pressed = modifiers.map(|key| keyboard.pressed(key).then_some(key));

    mouse.reset_all();
    mouse_wheel.clear();
    keyboard.reset_all();

    for key in pressed.into_iter().flatten() {
        keyboard.press(key);
    }
}

fn drop_egui_input_if_cursor_invisible(
    cursor_visible: Res<CursorVisible>,
    mut egui_inputs: Query<&mut bevy_egui::EguiInput>,
) {
    if cursor_visible.0 {
        return;
    }

    for mut egui_input in &mut egui_inputs {
        egui_input.events.clear();
    }
}

fn set_cursor_visible(
    cursor_visible: &mut CursorVisible,
    cursor_options: &mut CursorOptions,
    visible: bool,
) {
    cursor_options.grab_mode = if visible {
        CursorGrabMode::None
    } else {
        CursorGrabMode::Confined
    };
    cursor_options.visible = visible;
    cursor_visible.0 = visible;
}

fn cursor_grab(
    mut cursor_visible: ResMut<CursorVisible>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let Ok((window, mut cursor_options)) = windows.single_mut() else {
        return;
    };
    if !window.focused {
        return;
    }
    if keys.just_pressed(KeyCode::Tab) {
        let new_visible = !cursor_visible.0;
        set_cursor_visible(&mut cursor_visible, &mut cursor_options, new_visible);
    } else if keys.just_pressed(KeyCode::Escape) {
        set_cursor_visible(&mut cursor_visible, &mut cursor_options, true);
    } else if mouse.just_pressed(MouseButton::Right) {
        set_cursor_visible(&mut cursor_visible, &mut cursor_options, false);
    } else if mouse.just_released(MouseButton::Right) {
        set_cursor_visible(&mut cursor_visible, &mut cursor_options, true);
    }
}

fn reset_cursor_visibility_on_tool_change(
    mut cursor_visible: ResMut<CursorVisible>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    if let Ok((window, mut cursor_options)) = windows.single_mut() {
        if !window.focused {
            return;
        }
        set_cursor_visible(&mut cursor_visible, &mut cursor_options, true);
    }
}

pub fn color_picker(ui: &mut egui::Ui, color: &mut Color) {
    let mut egui_color = bevy_color_to_egui_hsv(*color);
    let original_alpha = color.alpha();
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = 220.0;
        egui::widgets::color_picker::color_picker_hsva_2d(
            ui,
            &mut egui_color,
            egui::color_picker::Alpha::Opaque,
        );
    });
    *color = egui_hsv_to_bevy_color(egui_color).with_alpha(original_alpha);
}
