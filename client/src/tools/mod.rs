use std::{fmt, ops::RangeInclusive, time::Duration};

use bevy::{input::mouse::MouseWheel, prelude::*};
use bevy_egui::egui;
use serde::{Deserialize, Serialize};

use crate::{
    color,
    ui::{normalize_scroll_wheel, CursorVisible},
    voxel::VoxelSizeMeters,
    AppState,
};

pub mod color_picker;
pub mod cube;
pub mod eraser;
pub mod floor;
pub mod prism;
pub mod spawn;
pub mod sphere;
pub mod tint;
pub mod wall;

pub fn plugin(app: &mut App) {
    app.add_plugins(Tool::plugin)
        .init_resource::<ActiveTool>()
        .init_resource::<ToolInUse>()
        .init_resource::<ToolColor>()
        .init_resource::<LastUsedColors>()
        .insert_resource(BrushSizeMinimum(0.0))
        .insert_resource(BrushSizeMaximum(0.0))
        .add_systems(Update, update_brush_sizes)
        .add_systems(Update, (update_tool_in_use, set_tool_on_input))
        .add_systems(PostUpdate, |mut active_tool: ResMut<ActiveTool>| {
            active_tool.update_previous();
        })
        .add_systems(OnExit(AppState::Edit), disable_tool_on_state_exit);
}

#[derive(Resource, Copy, Clone)]
pub struct BrushSizeMinimum(f32);
#[derive(Resource, Copy, Clone)]
pub struct BrushSizeMaximum(f32);

#[derive(Resource, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolColor {
    pub base: Color,
    pub color_noise: f32,
    pub grayscale_noise: f32,
}
impl Default for ToolColor {
    fn default() -> Self {
        Self {
            base: Color::WHITE,
            color_noise: 0.02,
            grayscale_noise: 0.10,
        }
    }
}
impl ToolColor {
    pub fn sample(&self) -> Color {
        let mut color = self.base;

        if self.color_noise > 0.0 {
            let base_color = color::bevy_color_to_vec3(color);
            let rand_color = Vec3::new(
                rand::random::<f32>(),
                rand::random::<f32>(),
                rand::random::<f32>(),
            );
            color = color::vec3_to_bevy_color(base_color.lerp(rand_color, self.color_noise));
        }
        if self.grayscale_noise > 0.0 {
            let mut hsv = Hsva::from(color);
            hsv.value = self.grayscale_noise * rand::random::<f32>()
                + (1.0 - self.grayscale_noise) * hsv.value;
            color = Color::from(hsv);
        }

        color
    }
}

#[derive(Resource, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LastUsedColors(pub [Color; 16]);
impl LastUsedColors {
    pub fn push(&mut self, color: Color) {
        if self.0[0] == color {
            return;
        }
        self.0.rotate_right(1);
        self.0[0] = color;
    }
}

macro_rules! generate_tool_enum {
    (
        $(($module:ident, $variant:ident, $display_name:expr, $description:expr, $icon:expr)),+ $(,)?
    ) => {
        #[derive(Resource, PartialEq, Eq, Debug, Clone, Copy, Default)]
        pub enum Tool {
            #[default]
            None,
            $($variant),+
        }
        impl Tool {
            /// Doesn't include `None`.
            pub const ALL: &'static [Tool] = &[$(Tool::$variant),+];
            pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
                let active_tool = world.resource::<ActiveTool>().tool;
                const HEIGHT: f32 = 40.0;

                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    let icon = active_tool.icon();
                    if !icon.is_empty() {
                        ui.add_sized(egui::vec2(0.0, HEIGHT), egui::Label::new(icon));
                    }
                    ui.add_sized(
                        egui::vec2(0.0, HEIGHT),
                        egui::Label::new(egui::RichText::new(active_tool.to_string()).underline())
                    );
                    ui.separator();

                    match active_tool {
                        Tool::None => { ui.label("Select a tool using the left panel or your number keys."); },
                        $(Tool::$variant => $module::ui_top_left_panel(ui, world)),+
                    }
                });
            }
            pub fn description(&self) -> &'static str {
                match self {
                    Tool::None => "",
                    $(Tool::$variant => $description),+
                }
            }
            pub fn icon(&self) -> &'static str {
                match self {
                    Tool::None => "",
                    $(Tool::$variant => $icon),+
                }
            }
            fn plugin(app: &mut App) {
                app.add_plugins(
                    (
                        $(
                            $module::plugin
                        ),+
                    )
                );
            }
        }
        impl fmt::Display for Tool {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{}",
                    match self {
                        Tool::None => "None",
                        $(Tool::$variant => $display_name),+
                    }
                )
            }
        }
    };
}
generate_tool_enum![
    (
        sphere,
        Sphere,
        "Sphere",
        "Paint with spheres.",
        egui_phosphor::regular::SPHERE
    ),
    (
        cube,
        Cube,
        "Cube",
        "Paint with cubes.",
        egui_phosphor::regular::CUBE
    ),
    (
        floor,
        Floor,
        "Floor",
        "Create floors.",
        egui_phosphor::regular::SQUARE
    ),
    (
        wall,
        Wall,
        "Wall",
        "Make walls.",
        egui_phosphor::regular::WALL
    ),
    (
        eraser,
        Eraser,
        "Eraser",
        "Erase your mistakes.",
        egui_phosphor::regular::ERASER
    ),
    (
        tint,
        Tint,
        "Tint",
        "Tint the world.",
        egui_phosphor::regular::PAINT_BRUSH
    ),
    (
        color_picker,
        ColorPicker,
        "Color Picker",
        "Borrow colors from the world.",
        egui_phosphor::regular::EYEDROPPER
    ),
    (
        spawn,
        Spawn,
        "Spawn",
        "Spawn objects into the world.",
        egui_phosphor::regular::BUILDINGS
    ),
    (
        prism,
        Prism,
        "Prism",
        "Reimagine reality.",
        egui_phosphor::regular::APERTURE
    ),
];

#[derive(Resource, PartialEq, Eq, Debug, Clone, Copy, Default)]
pub struct ActiveTool {
    tool: Tool,
    previous_tool: Tool,
}
impl ActiveTool {
    pub fn is_none(&self) -> bool {
        self.tool == Tool::None
    }
    pub fn is(&self, tool: Tool) -> bool {
        self.tool == tool
    }
    pub fn was(&self, tool: Tool) -> bool {
        self.previous_tool == tool
    }
    pub fn set_by_index(&mut self, index: usize) {
        let new_tool = Tool::ALL[index.clamp(0, Tool::ALL.len() - 1)];
        if new_tool != self.tool {
            self.tool = new_tool;
        } else {
            self.tool = Tool::None;
        }
    }
    pub fn disable_if_active(&mut self, tool: Tool) {
        if self.tool == tool {
            self.disable();
        }
    }
    pub fn disable(&mut self) {
        self.tool = Tool::None;
    }
    pub fn update_previous(&mut self) {
        self.previous_tool = self.tool;
    }
    pub fn tool_just_changed(&self) -> bool {
        self.tool != self.previous_tool
    }
}

#[derive(Resource, Default, Debug)]
pub struct ToolInUse {
    in_use: bool,
    previous_in_use: bool,
}
impl ToolInUse {
    pub fn set(&mut self, in_use: bool) {
        self.previous_in_use = self.in_use;
        self.in_use = in_use;
    }
}

// Run conditions
/// Run if the tool is the currently active tool.
pub fn is_active(tool: Tool) -> impl FnMut(Res<ActiveTool>) -> bool {
    move |active_tool: Res<ActiveTool>| active_tool.is(tool)
}
/// Run if the current tool just changed (including from `None`).
pub fn tool_just_changed(active_tool: Res<ActiveTool>) -> bool {
    active_tool.tool_just_changed()
}
/// Run if the tool just started being used (i.e. the tool use button just started being held down).
pub fn started_being_used(tool: Tool) -> impl Fn(Res<ToolInUse>, Res<ActiveTool>) -> bool {
    move |this: Res<ToolInUse>, active_tool: Res<ActiveTool>| {
        (this.in_use && !this.previous_in_use && active_tool.is(tool))
            || (this.in_use && !active_tool.was(tool) && active_tool.is(tool))
    }
}
/// Run if the tool just stopped being used (i.e. the tool use button just stopped being held down).
pub fn stopped_being_used(tool: Tool) -> impl Fn(Res<ToolInUse>, Res<ActiveTool>) -> bool {
    move |this: Res<ToolInUse>, active_tool: Res<ActiveTool>| {
        (!this.in_use && this.previous_in_use && active_tool.is(tool))
            || (this.in_use && active_tool.was(tool) && !active_tool.is(tool))
    }
}
/// Run if the tool is in use. Has an optional `frequency` parameter to run the system at a certain interval.
pub fn is_in_use(
    tool: Tool,
    frequency: Option<Duration>,
) -> impl FnMut(Res<ToolInUse>, Res<ActiveTool>, Res<Time>) -> bool {
    let mut timer = frequency.map(|frequency| Timer::new(frequency, TimerMode::Repeating));
    move |this: Res<ToolInUse>, active_tool: Res<ActiveTool>, time: Res<Time>| {
        if let Some(timer) = &mut timer {
            timer.tick(time.delta());
        }
        let timer_finished =
            timer.as_ref().is_none() || timer.as_ref().is_some_and(|timer| timer.just_finished());
        let in_use_now = this.in_use && active_tool.is(tool) && timer_finished;
        started_being_used(tool)(this, active_tool) || in_use_now
    }
}
/// Run if a brush should be created for this tool.
pub fn brush_should_be_created<T: Resource>(
    tool: Tool,
) -> impl FnMut(Option<Res<T>>, Res<ActiveTool>) -> bool {
    move |brush: Option<Res<T>>, active_tool: Res<ActiveTool>| {
        brush.is_none() && active_tool.is(tool)
    }
}
/// Run if a brush should be destroyed for this tool.
pub fn brush_should_be_destroyed<T: Resource>(
    tool: Tool,
) -> impl FnMut(Option<Res<T>>, Res<ActiveTool>) -> bool {
    move |brush: Option<Res<T>>, active_tool: Res<ActiveTool>| {
        brush.is_some() && !active_tool.is(tool)
    }
}

pub fn scroll_size_system<T: Resource>(
    tool: Tool,
    radius_extractor: impl Fn(&mut T) -> &mut f32,
) -> impl Fn(
    Res<ActiveTool>,
    Res<CursorVisible>,
    Res<BrushSizeMinimum>,
    Res<BrushSizeMaximum>,
    ResMut<T>,
    MessageReader<MouseWheel>,
) {
    const SCROLL_SENSITIVITY: f32 = 0.01;
    move |active_tool: Res<ActiveTool>,
          cursor_visible: Res<CursorVisible>,
          brush_size_minimum: Res<BrushSizeMinimum>,
          brush_size_maximum: Res<BrushSizeMaximum>,
          mut brush: ResMut<T>,
          mut scroll_events: MessageReader<MouseWheel>| {
        if !active_tool.is(tool) || !cursor_visible.0 {
            return;
        }
        let radius = radius_extractor(&mut brush);
        for event in scroll_events.read() {
            *radius += normalize_scroll_wheel(event) * SCROLL_SENSITIVITY;
        }
        *radius = radius.clamp(brush_size_minimum.0, brush_size_maximum.0);
    }
}

pub fn update_brush_sizes(
    voxel_size_meters: Res<VoxelSizeMeters>,
    mut brush_size_minimum: ResMut<BrushSizeMinimum>,
    mut brush_size_maximum: ResMut<BrushSizeMaximum>,
) {
    *brush_size_minimum = BrushSizeMinimum(voxel_size_meters.0);
    *brush_size_maximum = BrushSizeMaximum(voxel_size_meters.0 * 100.0);
}

pub fn brush_size_range(world: &World) -> RangeInclusive<f32> {
    let minimum = world.resource::<BrushSizeMinimum>().0;
    let maximum = world.resource::<BrushSizeMaximum>().0;
    minimum..=maximum
}

pub fn update_tool_in_use(
    mut tool_in_use: ResMut<ToolInUse>,
    mouse_button: Res<ButtonInput<MouseButton>>,
) {
    tool_in_use.set(mouse_button.pressed(MouseButton::Left));
}

pub fn ui_left_panel(ui: &mut egui::Ui, mut active_tool: ResMut<ActiveTool>) {
    for (idx, tool) in Tool::ALL.iter().copied().enumerate() {
        let button = egui::Button::new(format!("{} {tool}", tool.icon()))
            .shortcut_text(egui::RichText::from((idx + 1).to_string()).small())
            .selected(active_tool.is(tool));

        if ui
            .add_sized(egui::vec2(ui.available_width(), 32.), button)
            .on_hover_text(tool.description())
            .clicked()
        {
            active_tool.set_by_index(idx);
        }
    }
}

fn set_tool_on_input(mut active_tool: ResMut<ActiveTool>, keys: Res<ButtonInput<KeyCode>>) {
    let number_keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ];

    for (idx, key) in number_keys.iter().copied().enumerate() {
        if keys.just_pressed(key) {
            active_tool.set_by_index(idx);
        }
    }
}

fn disable_tool_on_state_exit(mut active_tool: ResMut<ActiveTool>) {
    active_tool.disable();
}
