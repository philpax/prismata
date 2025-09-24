use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::AppState;

use super::RemoveOnPlayExit;

pub mod rl;

pub fn plugin(app: &mut App) {
    app.add_event::<Spawn>()
        .insert_resource(PidActive(false))
        .init_resource::<PidSettings>()
        .add_plugins(rl::plugin)
        .add_systems(
            Update,
            (update_thruster_forces, thruster_pid_controller)
                .chain()
                .after(super::ui_top_left)
                .run_if(in_state(AppState::Play)),
        )
        .add_systems(Update, spawn_hoverplate);
}

#[derive(Resource, Default)]
pub struct PidActive(bool);

#[derive(Resource, Copy, Clone)]
pub struct PidSettings {
    pub kp: f32,
    pub ki: f32,
    pub kd: f32,
    pub target_distance: f32,
}
impl Default for PidSettings {
    fn default() -> Self {
        Self {
            kp: -12.0,
            ki: -3.0,
            kd: -2.0,
            target_distance: 1.0,
        }
    }
}
impl PidSettings {
    fn ui(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.kp, -100.0..=100.0).text("Kp"));
        ui.add(egui::Slider::new(&mut self.ki, -100.0..=100.0).text("Ki"));
        ui.add(egui::Slider::new(&mut self.kd, -100.0..=100.0).text("Kd"));
        ui.add(egui::Slider::new(&mut self.target_distance, 0.0..=10.0).text("Target distance"));
        if ui.button("Reset settings").clicked() {
            *self = Self::default();
        }
    }
}

#[derive(Component)]
pub struct Body;

#[derive(Component)]
pub struct Thruster(pub usize);

#[derive(Component)]
pub struct ThrusterPower(pub f32);

#[derive(Component)]
pub struct ThrusterAccumulatedError(f32);

#[derive(Component)]
pub struct ThrusterLastError(f32);

#[derive(Event)]
pub struct Spawn;

fn spawn_hoverplate(
    mut spawn_events: EventReader<Spawn>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    physical_object_query: Query<Entity, Or<(With<Body>, With<ThrusterPower>)>>,
) {
    if spawn_events.read().count() == 0 {
        return;
    }

    for entity in physical_object_query.iter() {
        commands.entity(entity).despawn_recursive();
    }

    let body_material = materials.add(Color::srgb(0.2, 0.7, 0.9));

    let body_width = 1.0;
    let body_height = 0.1;
    let body_depth = 2.0;

    let body_mesh = meshes.add(Cuboid::new(body_width, body_height, body_depth));
    let body_collider = Collider::cuboid(body_width, body_height, body_depth);

    let thruster_material = materials.add(Color::srgb(0.2, 0.2, 0.3));
    let thruster_radius = 0.2;
    let thruster_height = 0.1;
    let thruster_width_offset = body_width * 0.45;
    let thruster_height_offset = body_height * -0.25;
    let thruster_depth_offset = body_depth * 0.45;
    let thruster_mesh = meshes.add(Cylinder::new(thruster_radius, thruster_height));
    let thruster_collider = Collider::cylinder(thruster_radius, thruster_height);
    let thruster_properties = MassPropertiesBundle::new_computed(&thruster_collider, 1.0);

    let body_translation = Vec3::Y * (body_height + thruster_height) * 0.5
        + Vec3::new(
            rand::random::<f32>() * 2.0 - 1.0,
            0.0,
            rand::random::<f32>() * 2.0 - 1.0,
        );

    let body = commands
        .spawn((
            Body,
            PbrBundle {
                mesh: body_mesh.clone(),
                material: body_material.clone(),
                transform: Transform::from_translation(body_translation),
                ..default()
            },
            RigidBody::Dynamic,
            MassPropertiesBundle::new_computed(&body_collider, 1.0),
            LinearVelocity::ZERO,
            AngularVelocity::ZERO,
            AngularDamping(1.6),
            body_collider,
            RemoveOnPlayExit,
        ))
        .id();

    for (idx, (x, z)) in [
        (-thruster_width_offset, thruster_depth_offset),
        (thruster_width_offset, thruster_depth_offset),
        (-thruster_width_offset, -thruster_depth_offset),
        (thruster_width_offset, -thruster_depth_offset),
    ]
    .iter()
    .copied()
    .enumerate()
    {
        let translation = Vec3::new(x, thruster_height_offset, z);
        let thruster_id = commands
            .spawn((
                PbrBundle {
                    mesh: thruster_mesh.clone(),
                    material: thruster_material.clone(),
                    transform: Transform::from_translation(translation),
                    ..default()
                },
                RigidBody::Dynamic,
                thruster_properties.clone(),
                thruster_collider.clone(),
                ThrusterAccumulatedError(0.0),
                ThrusterLastError(0.0),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
                ExternalForce::new(Vec3::ZERO),
                Thruster(idx),
                ThrusterPower(rand::random::<f32>()),
                RayCaster::new(Vec3::ZERO, -Dir3::Y),
            ))
            .id();

        commands.entity(body).add_child(thruster_id);
        commands.spawn(FixedJoint::new(body, thruster_id).with_local_anchor_1(translation));
    }
}

fn update_thruster_forces(
    mut query: Query<(&GlobalTransform, &ThrusterPower, &mut ExternalForce)>,
) {
    for (thruster_transform, speed, mut external_force) in query.iter_mut() {
        let axis = Vec3::Y;
        *external_force = ExternalForce::new(
            thruster_transform.to_scale_rotation_translation().1 * (speed.0 * axis),
        );
    }
}

pub const THRUSTER_LIMIT_MAGNITUDE: f32 = 3.0;
pub(super) fn ui_top_left(
    ui: &mut egui::Ui,
    mut pid_active: ResMut<PidActive>,
    mut settings: ResMut<PidSettings>,
    mut thruster_query: Query<(&Thruster, &mut ThrusterPower, &RayHits)>,
    mut physical_object_query: Query<
        (
            Entity,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &mut ExternalForce,
            &mut ExternalImpulse,
            &mut ExternalTorque,
        ),
        (
            Or<(With<Body>, With<ThrusterPower>)>,
            Without<super::inverted_pendulum::Body>,
            Without<super::inverted_pendulum::WheelSpeed>,
        ),
    >,
    mut spawn_events: EventWriter<Spawn>,
    mut train_events: EventWriter<rl::StartTraining>,
    training_state: NonSendMut<rl::TrainingState>,
    config: ResMut<rl::DdpgConfig>,
    _commands: &mut Commands,
) {
    ui.horizontal(|ui| {
        if ui.button("Spawn").clicked() {
            spawn_events.send(Spawn);
        }
        if ui.button("Train").clicked() {
            train_events.send(rl::StartTraining);
        }
    });

    ui.horizontal(|ui| {
        rl::ui_top_left(ui, training_state, config);

        ui.separator();

        ui.vertical(|ui| {
            ui.set_min_width(200.0);

            let mut thrusters = thruster_query.iter_mut().collect::<Vec<_>>();
            thrusters.sort_by_key(|(wheel, _, _)| wheel.0);

            if ui
                .button("Reset Body Velocity")
                .interact(egui::Sense::click_and_drag())
                .dragged()
            {
                for (
                    _,
                    mut linear_velocity,
                    mut angular_velocity,
                    mut external_force,
                    mut external_impulse,
                    mut external_torque,
                ) in physical_object_query.iter_mut()
                {
                    *linear_velocity = LinearVelocity::default();
                    *angular_velocity = AngularVelocity::default();
                    *external_force = ExternalForce::default();
                    *external_impulse = ExternalImpulse::default();
                    *external_torque = ExternalTorque::default();
                }
            }

            let mut temp_pid_active = pid_active.0;
            ui.checkbox(&mut temp_pid_active, "PID Active");
            if temp_pid_active != pid_active.0 {
                pid_active.0 = temp_pid_active;
            }
            settings.ui(ui);
            for (thruster, mut speed, hits) in thrusters {
                ui.label(format!("Thruster {}", thruster.0));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(
                            &mut speed.0,
                            -THRUSTER_LIMIT_MAGNITUDE..=THRUSTER_LIMIT_MAGNITUDE,
                        )
                        .fixed_decimals(2),
                    );
                    ui.label(format!(
                        "{:.02?}",
                        hits.iter_sorted()
                            .map(|h| h.time_of_impact)
                            .filter(|d| *d > 0.0)
                            .collect::<Vec<_>>()
                    ));
                });
            }
        });
    });
}

fn thruster_pid_controller(
    pid_active: Res<PidActive>,
    settings: Res<PidSettings>,
    time: Res<Time<Physics>>,
    mut query: Query<(
        &mut ThrusterPower,
        &mut ThrusterAccumulatedError,
        &mut ThrusterLastError,
        &RayHits,
    )>,
) {
    if pid_active.is_changed() && !pid_active.0 {
        for (mut thruster_power, mut thruster_accumulated_error, mut thruster_last_error, _) in
            query.iter_mut()
        {
            thruster_power.0 = 0.0;
            thruster_accumulated_error.0 = 0.0;
            thruster_last_error.0 = 0.0;
        }
        return;
    }

    if !pid_active.0 {
        return;
    }

    let PidSettings {
        kp,
        ki,
        kd,
        target_distance,
    } = *settings;

    for (mut thruster_power, mut thruster_accumulated_error, mut thruster_last_error, hits) in
        query.iter_mut()
    {
        let Some(distance) = hits
            .iter_sorted()
            .map(|h| h.time_of_impact)
            .find(|d| *d > 0.0)
        else {
            continue;
        };

        let error = distance - target_distance;
        let delta = (error - thruster_last_error.0) / time.delta_seconds();
        let force = kp * error + ki * thruster_accumulated_error.0 + kd * delta;

        thruster_power.0 = force.clamp(-THRUSTER_LIMIT_MAGNITUDE, THRUSTER_LIMIT_MAGNITUDE);
        thruster_accumulated_error.0 += error * time.delta_seconds();
        thruster_last_error.0 = error;
    }
}
