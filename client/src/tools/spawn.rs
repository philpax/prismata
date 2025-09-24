use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use avian3d::prelude::{ColliderConstructor, ColliderConstructorHierarchy, RigidBody};
use bevy::{
    asset::io::AssetSourceId, prelude::*, render::view::RenderLayers,
    tasks::futures_lite::StreamExt,
};
use bevy_egui::{egui, EguiContexts};

use crate::{
    picking::PickableChildren,
    raycast::{update_world_rayhits, CursorRayHit, RaycastIgnore},
    rendering, voxel, AppState,
};

use super::{brush_should_be_destroyed, is_in_use, Tool};

#[derive(Component)]
pub struct SpawnableAssetPath(pub PathBuf);

#[derive(Bundle)]
pub struct SpawnableBundle {
    pub asset_path: SpawnableAssetPath,
    pub scene: SceneBundle,
    pub pickable_children: PickableChildren,
    pub render_layers: RenderLayers,
    pub propagate_render_layers: rendering::PropagateRenderLayers,
}
impl SpawnableBundle {
    pub fn new(assets: &AssetServer, asset_path: PathBuf, transform: Transform) -> Self {
        Self {
            asset_path: SpawnableAssetPath(asset_path.clone()),
            scene: SceneBundle {
                scene: assets.load(GltfAssetLabel::Scene(0).from_asset(asset_path)),
                transform,
                ..default()
            },
            pickable_children: PickableChildren,
            render_layers: RenderLayers::from_layers(&[
                rendering::ALL_NON_MASK_CAMERA_LAYER,
                rendering::MASK_CAMERA_ONLY_LAYER,
            ]),
            propagate_render_layers: rendering::PropagateRenderLayers,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.add_event::<SetSpawnPreview>()
        .add_systems(Startup, startup)
        .add_systems(
            Update,
            (
                on_preview_create,
                preview_update.run_if(resource_exists::<SpawnPreview>),
                on_preview_destroy.run_if(brush_should_be_destroyed::<SpawnPreview>(Tool::Spawn)),
                on_use.run_if(
                    is_in_use(Tool::Spawn, Some(Duration::from_millis(100)))
                        .and_then(resource_exists::<SpawnPreview>),
                ),
            )
                .chain()
                .after(update_world_rayhits),
        )
        .add_systems(OnEnter(AppState::Play), add_collider_to_spawnables);
}

fn startup(mut egui_contexts: EguiContexts, assets: Res<AssetServer>, mut commands: Commands) {
    let reader = assets
        .get_source(AssetSourceId::default())
        .unwrap()
        .reader();

    let paths = bevy::tasks::block_on(async move {
        reader
            .read_directory(std::path::Path::new(""))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    let mut asset_packs = categorize_assets(&assets, &mut egui_contexts, &paths);
    asset_packs.sort_by(|a, b| a.name.cmp(&b.name));

    commands.insert_resource(AssetPacks(asset_packs));
}

#[derive(Resource)]
struct SpawnPreview {
    preview_id: Entity,
    path: PathBuf,
}

#[derive(Resource)]
struct AssetPacks(Vec<AssetPack>);

#[derive(Event)]
struct SetSpawnPreview(PathBuf);

#[derive(Debug, Clone)]
struct AssetPack {
    name: String,
    spawnables: HashMap<String, Spawnable>,
}

#[derive(Debug, Clone)]
struct Spawnable {
    name: String,
    model: Option<PathBuf>,
    preview: Option<(PathBuf, Handle<Image>, egui::TextureId)>,
}

fn categorize_assets(
    assets: &AssetServer,
    egui_contexts: &mut EguiContexts,
    paths: &[PathBuf],
) -> Vec<AssetPack> {
    let mut asset_packs: HashMap<String, AssetPack> = HashMap::new();

    for path in paths {
        let components: Vec<&str> = path
            .components()
            .map(|c| c.as_os_str().to_str().unwrap())
            .collect();

        if components.len() != 3 {
            continue;
        }

        let pack_name = components[0].to_string();
        let category = components[1];
        let file_name = components[2];

        if category != "Models" && category != "Previews" {
            continue; // Skip if not a model or preview
        }

        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["png", "glb", "gltf"].contains(&e))
        {
            continue;
        }

        let asset_pack = asset_packs.entry(pack_name.clone()).or_insert(AssetPack {
            name: pack_name,
            spawnables: HashMap::new(),
        });

        let spawnable_name = Path::new(file_name)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let spawnable = asset_pack
            .spawnables
            .entry(spawnable_name.clone())
            .or_insert(Spawnable {
                name: spawnable_name,
                model: None,
                preview: None,
            });

        match category {
            "Models" => spawnable.model = Some(path.clone()),
            "Previews" => {
                spawnable.preview = {
                    let handle = assets.load(path.clone());
                    let texture_id = egui_contexts.add_image(handle.clone());
                    Some((path.clone(), handle, texture_id))
                }
            }
            _ => {} // This case should never occur due to the earlier check
        }
    }

    asset_packs.into_values().collect()
}

fn on_preview_create(
    cursor_ray_hit: Res<CursorRayHit>,
    assets: Res<AssetServer>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    spawn_preview: Option<Res<SpawnPreview>>,
    mut set_spawn_preview: EventReader<SetSpawnPreview>,
    mut commands: Commands,
) {
    let Some(set_spawn_preview) = set_spawn_preview.read().last() else {
        return;
    };

    if let Some(spawn_preview) = spawn_preview {
        commands
            .entity(spawn_preview.preview_id)
            .despawn_recursive();
    }

    let path = set_spawn_preview.0.clone();
    let preview_id = commands
        .spawn((
            SceneBundle {
                scene: assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone())),
                transform: Transform::from_translation(
                    cursor_ray_hit
                        .coords()
                        .map(|c| c.to_world(*voxels_per_meter))
                        .unwrap_or_default(),
                ),
                ..default()
            },
            RaycastIgnore,
        ))
        .id();
    commands.insert_resource(SpawnPreview { preview_id, path });
}

fn on_preview_destroy(spawn_preview: Res<SpawnPreview>, mut commands: Commands) {
    commands
        .entity(spawn_preview.preview_id)
        .despawn_recursive();
    commands.remove_resource::<SpawnPreview>();
}

fn preview_update(
    cursor_ray_hit: Res<CursorRayHit>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    spawn_preview: Res<SpawnPreview>,
    mut transform_query: Query<&mut Transform>,
) {
    if let Some(cursor_ray_hit) = cursor_ray_hit
        .coords()
        .map(|c| c.to_world(*voxels_per_meter))
    {
        if let Ok(mut transform) = transform_query.get_mut(spawn_preview.preview_id) {
            transform.translation = cursor_ray_hit;
        }
    }
}

fn on_use(
    assets: Res<AssetServer>,
    cursor_ray_hit: Res<CursorRayHit>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    spawn_preview: Res<SpawnPreview>,
    mut commands: Commands,
) {
    let Some(cursor_ray_hit) = cursor_ray_hit
        .coords()
        .map(|c| c.to_world(*voxels_per_meter))
    else {
        return;
    };

    commands.spawn(SpawnableBundle::new(
        &assets,
        spawn_preview.path.clone(),
        Transform::from_translation(cursor_ray_hit),
    ));
}

pub fn ui_top_left_panel(ui: &mut egui::Ui, world: &mut World) {
    let asset_packs = world.resource::<AssetPacks>();
    let images = world.resource::<Assets<Image>>();

    let mut spawn_preview_path = None;
    egui::menu::bar(ui, |ui| {
        for pack in &asset_packs.0 {
            ui.menu_button(pack.name.as_str(), |ui| {
                egui::containers::ScrollArea::vertical().show(ui, |ui| {
                    let mut values = pack.spawnables.values().collect::<Vec<_>>();
                    values.sort_by(|a, b| a.name.cmp(&b.name));
                    for spawnable in values {
                        let Some(path) = spawnable.model.as_ref() else {
                            continue;
                        };

                        let image = spawnable.preview.as_ref().map(|p| {
                            let size = images.get(&p.1).unwrap().size();
                            egui::Image::new(egui::load::SizedTexture::new(
                                p.2,
                                egui::vec2(size.x as f32, size.y as f32),
                            ))
                            .max_size(egui::vec2(64.0, 64.0))
                        });

                        if ui
                            .add(egui::Button::opt_image_and_text(
                                image,
                                Some(egui::WidgetText::from(spawnable.name.as_str())),
                            ))
                            .clicked()
                        {
                            spawn_preview_path = Some(path.clone());
                        }
                    }
                });
            });
        }
    });

    if let Some(path) = spawn_preview_path {
        world.send_event(SetSpawnPreview(path));
    }
}

fn add_collider_to_spawnables(
    mut commands: Commands,
    spawnables: Query<Entity, With<SpawnableAssetPath>>,
) {
    for entity in spawnables.iter() {
        commands.entity(entity).insert((
            RigidBody::Static,
            ColliderConstructorHierarchy::new(ColliderConstructor::ConvexHullFromMesh),
        ));
    }
}
