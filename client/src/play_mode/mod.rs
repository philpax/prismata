use std::time::Duration;

use avian3d::prelude::*;
use bevy::{prelude::*, time::common_conditions::on_timer};

use crate::{camera::MainCamera, AppState};

mod inverted_pendulum;

mod hoverplate;

#[derive(Component)]
pub struct RemoveOnPlayExit;

#[derive(Component)]
pub struct PreserveColliderOnPlayExit;

#[derive(Resource, PartialEq, Eq, Clone, Copy, Default)]
enum Agent {
    #[default]
    InvertedPendulum,
    Hoverplate,
}

pub fn plugin(app: &mut App) {
    app.add_plugins((inverted_pendulum::plugin, hoverplate::plugin))
        .insert_resource(CubeSize(0.05))
        .init_resource::<Agent>()
        .add_systems(
            Update,
            (
                ui_top_left,
                spawn_cubes_on_click.run_if(on_timer(Duration::from_secs_f32(0.05))),
            )
                .chain()
                .run_if(in_state(AppState::Play)),
        )
        .add_systems(
            OnExit(AppState::Play),
            (despawn_entities_on_play_exit, remove_colliders_on_play_exit).chain(),
        );
}

fn ui_top_left(
    mut egui_contexts: bevy_egui::EguiContexts,
    mut agent: ResMut<Agent>,

    wheel_query: Query<(
        &inverted_pendulum::Wheel,
        &mut inverted_pendulum::WheelSpeed,
    )>,
    body_query: Query<
        (
            Entity,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &mut ExternalForce,
            &mut ExternalImpulse,
            &mut ExternalTorque,
        ),
        (
            Or<(
                With<inverted_pendulum::Body>,
                With<inverted_pendulum::WheelSpeed>,
            )>,
            Without<hoverplate::Body>,
            Without<hoverplate::ThrusterPower>,
        ),
    >,
    wheel_axis: ResMut<inverted_pendulum::WheelAxis>,
    wheel_torque_transform_to_world: ResMut<inverted_pendulum::WheelTorqueTransformToWorld>,
    pendulum_spawns: EventWriter<inverted_pendulum::Spawn>,

    pid_active: ResMut<hoverplate::PidActive>,
    pid_settings: ResMut<hoverplate::PidSettings>,
    thruster_query: Query<(
        &hoverplate::Thruster,
        &mut hoverplate::ThrusterPower,
        &RayHits,
    )>,
    physical_object_query: Query<
        (
            Entity,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &mut ExternalForce,
            &mut ExternalImpulse,
            &mut ExternalTorque,
        ),
        (
            Or<(With<hoverplate::Body>, With<hoverplate::ThrusterPower>)>,
            Without<inverted_pendulum::Body>,
            Without<inverted_pendulum::WheelSpeed>,
        ),
    >,
    hoverplate_spawns: EventWriter<hoverplate::Spawn>,
    (hoverplate_rl_start_training, hoverplate_rl_training_state, hoverplate_rl_config): (
        EventWriter<hoverplate::rl::StartTraining>,
        NonSendMut<hoverplate::rl::TrainingState>,
        ResMut<hoverplate::rl::DdpgConfig>,
    ),

    mut commands: Commands,

    mut cube_size: ResMut<CubeSize>,
    mut store: ResMut<GizmoConfigStore>,
) {
    use bevy_egui::egui;
    egui::Area::new("PlayModeTopLeft".into())
        .anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
        .show(egui_contexts.ctx_mut(), |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.checkbox(
                    &mut store.config_mut::<PhysicsGizmos>().0.enabled,
                    "Physics Gizmos",
                );

                ui.label("Cube Size");
                ui.add(egui::Slider::new(&mut cube_size.0, 0.01..=1.0).fixed_decimals(2));

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut *agent, Agent::InvertedPendulum, "Inverted Pendulum");
                    ui.selectable_value(&mut *agent, Agent::Hoverplate, "Hoverplate");
                });

                match *agent {
                    Agent::InvertedPendulum => {
                        inverted_pendulum::ui_top_left(
                            ui,
                            wheel_query,
                            body_query,
                            wheel_axis,
                            wheel_torque_transform_to_world,
                            pendulum_spawns,
                            &mut commands,
                        );
                    }
                    Agent::Hoverplate => {
                        hoverplate::ui_top_left(
                            ui,
                            pid_active,
                            pid_settings,
                            thruster_query,
                            physical_object_query,
                            hoverplate_spawns,
                            hoverplate_rl_start_training,
                            hoverplate_rl_training_state,
                            hoverplate_rl_config,
                            &mut commands,
                        );
                    }
                }
            });
        });
}

fn despawn_entities_on_play_exit(
    mut commands: Commands,
    query: Query<Entity, With<RemoveOnPlayExit>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn_recursive();
    }
}

fn remove_colliders_on_play_exit(
    mut commands: Commands,
    query: Query<Entity, (With<Collider>, Without<PreserveColliderOnPlayExit>)>,
) {
    for entity in query.iter() {
        commands.entity(entity).remove::<(
            ColliderConstructor,
            Collider,
            ColliderParent,
            ColliderAabb,
            CollidingEntities,
            ColliderDensity,
            ColliderMassProperties,
            RigidBody,
        )>();
    }
}

#[derive(Resource)]
struct CubeSize(f32);

fn spawn_cubes_on_click(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
    left_click: Res<ButtonInput<MouseButton>>,
    cube_size: Res<CubeSize>,
) {
    let cube_mesh = meshes.add(Cuboid::default());
    let cube_material = materials.add(Color::hsl(
        rand::random::<f32>() * 360.0,
        0.7 + rand::random::<f32>() * 0.2,
        0.5 + rand::random::<f32>() * 0.2,
    ));

    let camera_transform = camera.single();

    if left_click.pressed(MouseButton::Left) {
        let (_, rotation, translation) = camera_transform.to_scale_rotation_translation();
        let direction = rotation * -Vec3::Z;
        let position = translation + direction * 0.1;
        commands.spawn((
            PbrBundle {
                mesh: cube_mesh.clone(),
                material: cube_material.clone(),
                transform: Transform::from_translation(position)
                    .with_scale(Vec3::splat(cube_size.0))
                    .with_rotation(rotation),
                ..default()
            },
            RigidBody::Dynamic,
            Collider::cuboid(1.0, 1.0, 1.0),
            LinearVelocity(direction * 5.0),
            RemoveOnPlayExit,
        ));
    }
}
