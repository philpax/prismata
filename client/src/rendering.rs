use std::f32::consts::TAU;

use avian3d::prelude::{ColliderConstructor, RigidBody};
use bevy::{
    color::palettes::css::SILVER,
    prelude::*,
    render::view::{Layer, RenderLayers},
};
use serde::{Deserialize, Serialize};

use crate::{play_mode::PreserveColliderOnPlayExit, raycast::RaycastIgnore};

#[derive(Component)]
pub struct Floor;

#[derive(Resource, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FloorColor(pub Color);

#[derive(Component)]
pub struct Sun;

#[derive(Resource, Clone, Copy, Deref, DerefMut, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SunAngle(pub f32);

#[derive(Component, Debug)]
/// When attached, the entity will pulse its alpha value over time.
pub struct AlphaPulse {
    /// The size of the pulse, 0-1.
    pub magnitude: f32,
    /// The period of a pulse in seconds.
    pub period: f32,
}
impl AlphaPulse {
    pub fn new(magnitude: f32, period: f32) -> Self {
        Self { magnitude, period }
    }
}

#[derive(Component, Debug)]
/// A line will be rendered from `p0` to `p1` in `color` for as long as the entity with this component exists.
///
/// Can spawn multiple of these.
pub struct DebugLine {
    pub p0: Vec3,
    pub p1: Vec3,
    pub color: Color,
}
impl DebugLine {
    #[allow(dead_code)]
    pub fn new(p0: Vec3, p1: Vec3, color: Color) -> Self {
        Self { p0, p1, color }
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct MainCameraGizmos {}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct MainCameraGizmosWithoutDepth {}

pub const ALL_NON_MASK_CAMERA_LAYER: Layer = 0;
pub const MAIN_CAMERA_ONLY_LAYER: Layer = 1;
pub const MASK_CAMERA_ONLY_LAYER: Layer = 2;

pub fn draw_gizmo_cross<Config: GizmoConfigGroup, Clear: 'static + Send + Sync>(
    gizmos: &mut Gizmos<Config, Clear>,
    position: Vec3,
    size: f32,
) {
    gizmos.line(
        position - Vec3::X * size,
        position + Vec3::X * size,
        Color::linear_rgb(1.0, 0.0, 0.0),
    );

    gizmos.line(
        position - Vec3::Y * size,
        position + Vec3::Y * size,
        Color::linear_rgb(0.0, 1.0, 0.0),
    );

    gizmos.line(
        position - Vec3::Z * size,
        position + Vec3::Z * size,
        Color::linear_rgb(0.0, 0.0, 1.0),
    );
}

pub fn plugin(app: &mut App) {
    app.insert_gizmo_config(
        MainCameraGizmos::default(),
        GizmoConfig {
            render_layers: RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER),
            ..default()
        },
    )
    .insert_gizmo_config(
        MainCameraGizmosWithoutDepth::default(),
        GizmoConfig {
            render_layers: RenderLayers::layer(MAIN_CAMERA_ONLY_LAYER),
            depth_bias: -1.0,
            ..default()
        },
    )
    .add_systems(Startup, setup)
    .add_systems(Update, (update_floor_color, render_debug_lines))
    .add_systems(
        Update,
        (
            propagate_render_layers_added,
            propagate_render_layers_changed,
            propagate_render_layers_add_child,
        )
            .chain(),
    );

    #[cfg(feature = "webgpu")]
    app.add_systems(Update, update_sun);

    app.add_systems(PostUpdate, run_alpha_pulse);
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    const PLANE_SIZE: f32 = 1000.0;

    // Sun
    commands.spawn((
        DirectionalLightBundle {
            ..Default::default()
        },
        Sun, // Marks the light as Sun
    ));
    commands.insert_resource(SunAngle(90.0f32.to_radians()));

    // ground plane
    let base_color = Color::from(SILVER);
    commands.spawn((
        Floor,
        PbrBundle {
            mesh: meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(PLANE_SIZE, PLANE_SIZE)
                    .subdivisions(1),
            ),
            material: materials.add(base_color),
            ..default()
        },
        RaycastIgnore,
        ColliderConstructor::TrimeshFromMesh,
        RigidBody::Static,
        PreserveColliderOnPlayExit,
    ));
    commands.insert_resource(FloorColor(base_color));
}

fn run_alpha_pulse(
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    tool_brush: Query<(&AlphaPulse, &Handle<StandardMaterial>)>,
) {
    for (AlphaPulse { magnitude, period }, material) in tool_brush.iter() {
        let material = materials.get_mut(material).unwrap();
        material.base_color.set_alpha(
            (1.0 - *magnitude) + *magnitude * (time.elapsed_seconds() * TAU / *period).sin(),
        );
    }
}

fn update_floor_color(
    floor: Query<&Handle<StandardMaterial>, With<Floor>>,
    floor_color: Res<FloorColor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(floor_material) = floor.get_single() else {
        return;
    };
    let material = materials.get_mut(floor_material.id()).unwrap();
    material.base_color = floor_color.0;
}

#[cfg(feature = "webgpu")]
fn update_sun(
    mut atmosphere: bevy_atmosphere::prelude::AtmosphereMut<bevy_atmosphere::prelude::Nishita>,
    mut query: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
    sun_angle: Res<SunAngle>,
) {
    use light_consts::lux::AMBIENT_DAYLIGHT;
    let Ok((mut light_trans, mut directional)) = query.get_single_mut() else {
        return;
    };
    let t = sun_angle.0;
    atmosphere.sun_position = Vec3::new(0., t.sin(), t.cos());
    light_trans.rotation = Quat::from_rotation_x(-t);
    directional.illuminance = t.sin().max(0.0).powf(2.0) * AMBIENT_DAYLIGHT;
}

fn render_debug_lines(mut gizmos: Gizmos, query: Query<&DebugLine>) {
    for line in query.iter() {
        gizmos.line(line.p0, line.p1, line.color);
    }
}

// https://github.com/bevyengine/bevy/issues/5183#issuecomment-1555946084
#[derive(Component)]
pub struct PropagateRenderLayers;

fn propagate_render_layers_added(
    mut cmds: Commands,
    parents: Query<(&RenderLayers, &Children), (Added<PropagateRenderLayers>, With<Children>)>,
) {
    for (render_layers, children) in &parents {
        for child in children {
            cmds.entity(*child)
                .insert((PropagateRenderLayers, render_layers.clone()));
        }
    }
}

fn propagate_render_layers_changed(
    mut cmds: Commands,
    parents: Query<
        (&RenderLayers, &Children),
        (
            With<PropagateRenderLayers>,
            With<Children>,
            Changed<RenderLayers>,
        ),
    >,
) {
    for (render_layers, children) in &parents {
        for child in children {
            cmds.entity(*child).insert(render_layers.clone());
        }
    }
}

fn propagate_render_layers_add_child(
    mut cmds: Commands,
    parents: Query<(&RenderLayers, &Children), (With<PropagateRenderLayers>, Changed<Children>)>,
) {
    for (render_layers, children) in &parents {
        for child in children {
            cmds.entity(*child)
                .insert((PropagateRenderLayers, render_layers.clone()));
        }
    }
}
