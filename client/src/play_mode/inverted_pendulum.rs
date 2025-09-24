use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::AppState;

use super::RemoveOnPlayExit;

pub fn plugin(app: &mut App) {
    app.init_resource::<WheelAxis>()
        .init_resource::<WheelTorqueTransformToWorld>()
        .add_event::<Spawn>()
        .add_systems(
            Update,
            update_wheel_speeds
                .after(super::ui_top_left)
                .run_if(in_state(AppState::Play)),
        )
        .add_systems(Update, spawn_inverted_pendulum);
}

#[derive(Component)]
pub struct Body;

#[derive(Component)]
pub struct Wheel(usize);

#[derive(Resource, Default, Debug, PartialEq, Eq, Copy, Clone)]
pub enum WheelAxis {
    #[default]
    X,
    Y,
    Z,
}
impl From<WheelAxis> for Vec3 {
    fn from(axis: WheelAxis) -> Vec3 {
        match axis {
            WheelAxis::X => Vec3::X,
            WheelAxis::Y => Vec3::Y,
            WheelAxis::Z => Vec3::Z,
        }
    }
}

#[derive(Resource, Debug)]
pub struct WheelTorqueTransformToWorld(bool);
impl Default for WheelTorqueTransformToWorld {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Component)]
pub struct WheelSpeed(f32);

#[derive(Event)]
pub struct Spawn;

fn spawn_inverted_pendulum(
    mut spawn_events: EventReader<Spawn>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    body_query: Query<(), With<Body>>,
) {
    for _ in spawn_events.read() {
        if body_query.iter().next().is_some() {
            return;
        }

        let body_material = materials.add(Color::srgb(0.2, 0.7, 0.9));
        let wheel_material = materials.add(Color::srgb(0.2, 0.2, 0.3));

        let body_width = 1.0;
        let body_height = 2.0;
        let body_thickness = 0.25;

        let body_translation = Vec3::Y * 2.5;
        let body_mesh = meshes.add(Cuboid::new(body_width, body_height, body_thickness));
        let body_collider = Collider::cuboid(body_width, body_height, body_thickness);
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

        let wheel_radius = 0.5;
        let wheel_width = 0.15;
        let wheel_height_offset = -body_height / 2.0 + wheel_radius * 0.5;
        let wheel_body_offset = 0.05;

        let wheel_mesh = meshes.add(Cylinder::new(wheel_radius, wheel_width));
        let wheel_collider = Collider::cylinder(wheel_radius, wheel_width);
        let wheel_properties = MassPropertiesBundle::new_computed(&wheel_collider, 1.0);

        for (idx, offset) in [
            Vec3::new(
                (body_width + wheel_width + wheel_body_offset) / 2.0,
                wheel_height_offset,
                0.0,
            ),
            Vec3::new(
                -(body_width + wheel_width + wheel_body_offset) / 2.0,
                wheel_height_offset,
                0.0,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let tyre_rotation_transform = Transform {
                rotation: Quat::from_rotation_z(90.0_f32.to_radians()),
                ..Default::default()
            };
            let wheel = commands
                .spawn((
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    RigidBody::Dynamic,
                    wheel_properties.clone(),
                    Collider::compound(vec![(
                        Vec3::ZERO,
                        tyre_rotation_transform,
                        wheel_collider.clone(),
                    )]),
                    ExternalTorque::new(Vec3::ZERO),
                    Wheel(idx),
                    WheelSpeed(0.0),
                    RemoveOnPlayExit,
                ))
                .with_children(|children| {
                    children.spawn((PbrBundle {
                        mesh: wheel_mesh.clone(),
                        material: wheel_material.clone(),
                        transform: tyre_rotation_transform,
                        ..default()
                    },));
                })
                .id();
            commands.spawn((
                RevoluteJoint::new(body, wheel)
                    .with_local_anchor_1(offset)
                    .with_aligned_axis(Vec3::Z),
                RemoveOnPlayExit,
            ));
        }
    }
}

fn update_wheel_speeds(
    mut query: Query<(&GlobalTransform, &WheelSpeed, &mut ExternalTorque)>,
    wheel_axis: Res<WheelAxis>,
    wheel_torque_transform_to_world: Res<WheelTorqueTransformToWorld>,
) {
    for (wheel_transform, speed, mut external_torque) in query.iter_mut() {
        let axis: Vec3 = (*wheel_axis).into();
        let torque = speed.0 * axis;
        let torque = if wheel_torque_transform_to_world.0 {
            wheel_transform.transform_point(torque)
        } else {
            torque
        };
        *external_torque = ExternalTorque::new(torque);
    }
}

pub(super) fn ui_top_left(
    ui: &mut egui::Ui,
    mut wheel_query: Query<(&Wheel, &mut WheelSpeed)>,
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
            Or<(With<Body>, With<WheelSpeed>)>,
            Without<super::hoverplate::Body>,
            Without<super::hoverplate::ThrusterPower>,
        ),
    >,
    mut wheel_axis: ResMut<WheelAxis>,
    mut wheel_torque_transform_to_world: ResMut<WheelTorqueTransformToWorld>,
    mut events: EventWriter<Spawn>,
    commands: &mut Commands,
) {
    ui.horizontal(|ui| {
        if ui.button("Spawn").clicked() {
            events.send(Spawn);
        }
        if ui.button("Despawn").clicked() {
            for (entity, _, _, _, _, _) in physical_object_query.iter() {
                commands.entity(entity).despawn_recursive();
            }
        }
    });

    let mut wheels = wheel_query.iter_mut().collect::<Vec<_>>();
    wheels.sort_by_key(|(wheel, _)| wheel.0);

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

    ui.horizontal(|ui| {
        ui.label("Torque Axis");
        ui.radio_value(&mut *wheel_axis, WheelAxis::X, "X");
        ui.radio_value(&mut *wheel_axis, WheelAxis::Y, "Y");
        ui.radio_value(&mut *wheel_axis, WheelAxis::Z, "Z");
        ui.checkbox(&mut wheel_torque_transform_to_world.0, "Transform to World");
    });

    const MAGNITUDE: f32 = 10.0;
    for (wheel, mut speed) in wheels {
        ui.label(format!("Wheel {}", wheel.0));
        ui.add(egui::Slider::new(&mut speed.0, -MAGNITUDE..=MAGNITUDE).fixed_decimals(2));
    }
}
