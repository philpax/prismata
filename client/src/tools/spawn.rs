use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use avian3d::prelude::{ColliderConstructor, ColliderConstructorHierarchy, RigidBody};
use bevy::{
    asset::io::AssetSourceId, camera::visibility::RenderLayers, picking::prelude::Pickable,
    prelude::*, tasks::futures_lite::StreamExt,
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
    pub scene: SceneRoot,
    pub transform: Transform,
    pub pickable_children: PickableChildren,
    pub render_layers: RenderLayers,
    pub propagate_render_layers: rendering::PropagateRenderLayers,
}
impl SpawnableBundle {
    pub fn new(assets: &AssetServer, asset_path: PathBuf, transform: Transform) -> Self {
        Self {
            asset_path: SpawnableAssetPath(asset_path.clone()),
            scene: SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(asset_path))),
            transform,
            pickable_children: PickableChildren,
            render_layers: RenderLayers::from_layers(&[
                rendering::ALL_NON_MASK_CAMERA_LAYER as usize,
                rendering::MASK_CAMERA_ONLY_LAYER as usize,
            ]),
            propagate_render_layers: rendering::PropagateRenderLayers,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.add_message::<SetSpawnPreview>()
        .add_systems(Startup, startup)
        .add_systems(
            Update,
            (
                register_preview_textures.run_if(resource_exists::<PendingTextureRegistration>),
                on_preview_create,
                preview_update.run_if(resource_exists::<SpawnPreview>),
                on_preview_destroy.run_if(brush_should_be_destroyed::<SpawnPreview>(Tool::Spawn)),
                on_use.run_if(
                    is_in_use(Tool::Spawn, Some(Duration::from_millis(100)))
                        .and(resource_exists::<SpawnPreview>),
                ),
            )
                .chain()
                .after(update_world_rayhits),
        )
        .add_systems(OnEnter(AppState::Play), add_collider_to_spawnables);
}

fn startup(assets: Res<AssetServer>, mut commands: Commands) {
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
    let mut asset_packs = categorize_assets(&assets, &paths);
    asset_packs.sort_by(|a, b| a.name.cmp(&b.name));

    commands.insert_resource(AssetPacks(asset_packs));
    commands.insert_resource(PendingTextureRegistration);
}

#[derive(Resource)]
struct SpawnPreview {
    preview_id: Entity,
    path: PathBuf,
}

#[derive(Resource)]
struct AssetPacks(Vec<AssetPack>);

/// Marker resource indicating textures still need to be registered with egui.
#[derive(Resource)]
struct PendingTextureRegistration;

#[derive(Message)]
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
    /// Preview image - texture_id is None until registered with egui context.
    preview: Option<(PathBuf, Handle<Image>, Option<egui::TextureId>)>,
}

fn categorize_assets(assets: &AssetServer, paths: &[PathBuf]) -> Vec<AssetPack> {
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
                let handle = assets.load(path.clone());
                // texture_id will be registered later once egui context is available
                spawnable.preview = Some((path.clone(), handle, None));
            }
            _ => {} // This case should never occur due to the earlier check
        }
    }

    asset_packs.into_values().collect()
}

/// Registers preview textures with the egui context once it's available.
fn register_preview_textures(
    mut egui_contexts: EguiContexts,
    mut asset_packs: ResMut<AssetPacks>,
    mut commands: Commands,
) {
    // Check if egui context is available
    let Ok(ctx) = egui_contexts.ctx_mut() else {
        return;
    };
    let _ = ctx; // Just needed to verify context exists

    for pack in &mut asset_packs.0 {
        for spawnable in pack.spawnables.values_mut() {
            if let Some((_, handle, texture_id @ None)) = &mut spawnable.preview {
                *texture_id = Some(
                    egui_contexts
                        .add_image(bevy_egui::EguiTextureHandle::Strong(handle.clone())),
                );
            }
        }
    }

    // Remove the marker once all textures are registered
    commands.remove_resource::<PendingTextureRegistration>();
}

fn on_preview_create(
    cursor_ray_hit: Res<CursorRayHit>,
    assets: Res<AssetServer>,
    voxels_per_meter: Res<voxel::VoxelsPerMeter>,
    spawn_preview: Option<Res<SpawnPreview>>,
    mut set_spawn_preview: MessageReader<SetSpawnPreview>,
    mut commands: Commands,
) {
    let Some(set_spawn_preview) = set_spawn_preview.read().last() else {
        return;
    };

    if let Some(spawn_preview) = spawn_preview {
        commands.entity(spawn_preview.preview_id).despawn();
    }

    let path = set_spawn_preview.0.clone();
    let preview_id = commands
        .spawn((
            SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))),
            Transform::from_translation(
                cursor_ray_hit
                    .coords()
                    .map(|c| c.to_world(*voxels_per_meter))
                    .unwrap_or_default(),
            ),
            RaycastIgnore,
            Pickable::IGNORE,
        ))
        .id();
    commands.insert_resource(SpawnPreview { preview_id, path });
}

fn on_preview_destroy(spawn_preview: Res<SpawnPreview>, mut commands: Commands) {
    commands.entity(spawn_preview.preview_id).despawn();
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

    if asset_packs.0.is_empty() {
        ui.label("No asset packs found.");
        return;
    }

    let mut spawn_preview_path = None;
    for pack in &asset_packs.0 {
        ui.menu_button(pack.name.as_str(), |ui| {
            egui::containers::ScrollArea::vertical().show(ui, |ui| {
                let mut values = pack.spawnables.values().collect::<Vec<_>>();
                values.sort_by(|a, b| a.name.cmp(&b.name));
                for spawnable in values {
                    let Some(path) = spawnable.model.as_ref() else {
                        continue;
                    };

                    // Only show image if texture_id has been registered
                    let image = spawnable
                        .preview
                        .as_ref()
                        .and_then(|(_, handle, texture_id)| {
                            let texture_id = (*texture_id)?;
                            let size = images.get(handle)?.size();
                            Some(
                                egui::Image::new(egui::load::SizedTexture::new(
                                    texture_id,
                                    egui::vec2(size.x as f32, size.y as f32),
                                ))
                                .max_size(egui::vec2(64.0, 64.0)),
                            )
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

    if let Some(path) = spawn_preview_path {
        world.write_message(SetSpawnPreview(path));
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
