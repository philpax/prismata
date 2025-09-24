use std::{
    io::{Read, Seek, Write},
    path::PathBuf,
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use zip::result::ZipError;

use crate::{
    file_picker,
    rendering::{FloorColor, SunAngle},
    tools::{
        prism::PrismPaintSettings,
        spawn::{SpawnableAssetPath, SpawnableBundle},
        LastUsedColors, ToolColor,
    },
    voxel::{self, VoxelsPerMeter},
    ProjectName,
};

pub fn plugin(app: &mut App) {
    app.add_plugins((
        file_picker::ReadPlugin::new(WorldLoadHandler),
        file_picker::WritePlugin::new(WorldSaveHandler),
    ));
}

#[derive(Debug)]
enum LoadSaveError {
    Io(std::io::Error),
    Zip(zip::result::ZipError),
    Serde(serde_json::Error),
}
impl From<std::io::Error> for LoadSaveError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
impl From<zip::result::ZipError> for LoadSaveError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Zip(err)
    }
}
impl From<serde_json::Error> for LoadSaveError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err)
    }
}
impl std::fmt::Display for LoadSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "IO error: {err}"),
            Self::Zip(err) => write!(f, "ZIP error: {err}"),
            Self::Serde(err) => write!(f, "Serde error: {err}"),
        }
    }
}
impl std::error::Error for LoadSaveError {}
type LoadSaveResult<T> = Result<T, LoadSaveError>;

#[derive(Serialize, Deserialize)]
struct Meta {
    project_name: ProjectName,
    tool_color: ToolColor,
    last_used_colors: LastUsedColors,
    voxels_per_meter: VoxelsPerMeter,
    floor_color: FloorColor,
    sun_angle: SunAngle,
    prism_paint_settings: PrismPaintSettings,
}
#[allow(clippy::clone_on_copy)]
impl Meta {
    const FILENAME: &'static str = "meta.json";

    fn load_from_zip<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> LoadSaveResult<Meta> {
        Ok(serde_json::from_reader(zip.by_name(Self::FILENAME)?)?)
    }
    fn save_to_zip<W: Write + Seek>(
        zip: &mut zip::ZipWriter<W>,
        world: &mut World,
        options: zip::write::SimpleFileOptions,
    ) -> LoadSaveResult<()> {
        zip.start_file(Meta::FILENAME, options)?;
        serde_json::to_writer(zip, &Self::extract_from_world(world))?;
        Ok(())
    }

    fn extract_from_world(world: &World) -> Self {
        Self {
            project_name: world.resource::<ProjectName>().clone(),
            tool_color: world.resource::<ToolColor>().clone(),
            last_used_colors: world.resource::<LastUsedColors>().clone(),
            voxels_per_meter: world.resource::<VoxelsPerMeter>().clone(),
            floor_color: world.resource::<FloorColor>().clone(),
            sun_angle: world.resource::<SunAngle>().clone(),
            prism_paint_settings: world.resource::<PrismPaintSettings>().clone(),
        }
    }
    fn apply_to_world(self, world: &mut World) {
        world.insert_resource(self.project_name.clone());
        world.insert_resource(self.tool_color.clone());
        world.insert_resource(self.last_used_colors.clone());
        world.insert_resource(self.voxels_per_meter.clone());
        // HACK: Insert the old voxels per meter resource to avoid tripping
        // `update_sizes`'s clear_all
        world.insert_resource(voxel::OldVoxelsPerMeter(self.voxels_per_meter.0));
        world.insert_resource(self.floor_color.clone());
        world.insert_resource(self.sun_angle.clone());
        world.insert_resource(self.prism_paint_settings.clone());
    }
}

struct ChunkCoordsAndData(pub Vec<(voxel::ChunkCoords, voxel::ChunkData)>);
impl ChunkCoordsAndData {
    fn load_from_zip<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> LoadSaveResult<Self> {
        let mut chunk_coords_and_data = vec![];

        let chunk_coords_and_filenames: Vec<_> = zip
            .file_names()
            .filter_map(|filename| {
                let coords = filename
                    .strip_prefix("chunks/chunk_")
                    .and_then(|s| s.strip_suffix(".bin"))
                    .and_then(|s| {
                        let mut parts = s.split('_');
                        Some(voxel::ChunkCoords(IVec3 {
                            x: parts.next()?.parse().ok()?,
                            y: parts.next()?.parse().ok()?,
                            z: parts.next()?.parse().ok()?,
                        }))
                    })?;
                Some((coords, filename.to_string()))
            })
            .collect();

        for (coords, filename) in chunk_coords_and_filenames {
            let mut data = voxel::ChunkData::default();
            let mut file = zip.by_name(&filename)?;

            for i in 0..voxel::VOXELS_PER_CHUNK_SIDE.pow(3) {
                let mut voxel_buf: [u8; 4] = [0; 4];
                file.read_exact(&mut voxel_buf)?;
                data.voxels[i] = voxel::Voxel::from_u8_array(voxel_buf);
            }
            chunk_coords_and_data.push((coords, data));
        }

        Ok(Self(chunk_coords_and_data))
    }
    fn save_to_zip<W: Write + Seek>(
        zip: &mut zip::ZipWriter<W>,
        world: &mut World,
        options: zip::write::SimpleFileOptions,
    ) -> LoadSaveResult<()> {
        for (coords, data) in world
            .query::<(&voxel::ChunkCoords, &voxel::ChunkData)>()
            .iter(world)
        {
            zip.start_file(
                format!("chunks/chunk_{}_{}_{}.bin", coords.x, coords.y, coords.z),
                options,
            )?;
            for voxel in data.iter() {
                zip.write_all(&voxel.as_u8_array())?;
            }
        }
        Ok(())
    }

    fn apply_to_world(self, world: &mut World) {
        voxel::clear_all(world);
        for (coords, data) in self.0 {
            world.spawn(voxel::ChunkBundle::new(coords, Some(data), vec![]));
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Spawnable {
    asset_path: PathBuf,
    transform: Transform,
}
#[derive(Serialize, Deserialize, Default)]
struct Spawnables {
    spawnables: Vec<Spawnable>,
}
impl Spawnables {
    const FILENAME: &'static str = "spawnables.json";

    fn load_from_zip<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> LoadSaveResult<Self> {
        match zip.by_name(Spawnables::FILENAME) {
            Ok(file) => Ok(serde_json::from_reader(file)?),
            Err(ZipError::FileNotFound) => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }
    fn save_to_zip<W: Write + Seek>(
        zip: &mut zip::ZipWriter<W>,
        world: &mut World,
        options: zip::write::SimpleFileOptions,
    ) -> LoadSaveResult<()> {
        zip.start_file(Spawnables::FILENAME, options)?;
        serde_json::to_writer(zip, &Self::extract_from_world(world))?;
        Ok(())
    }

    fn extract_from_world(world: &mut World) -> Self {
        let mut spawnables = vec![];
        for (transform, path) in world
            .query::<(&Transform, &SpawnableAssetPath)>()
            .iter(world)
        {
            spawnables.push(Spawnable {
                asset_path: path.0.clone(),
                transform: *transform,
            });
        }
        Self { spawnables }
    }
    fn apply_to_world(self, world: &mut World) {
        let existing_spawnables = world
            .query_filtered::<Entity, With<SpawnableAssetPath>>()
            .iter(world)
            .collect::<Vec<_>>();
        for entity in existing_spawnables {
            world.despawn(entity);
        }

        let assets = world.resource::<AssetServer>().clone();
        for spawnable in &self.spawnables {
            world.spawn(SpawnableBundle::new(
                &assets,
                spawnable.asset_path.clone(),
                spawnable.transform,
            ));
        }
    }
}

#[derive(Event)]
pub struct RequestLoad;
#[derive(Clone, Copy)]
struct WorldLoadHandler;
impl file_picker::ReadHandler for WorldLoadHandler {
    type Event = RequestLoad;

    fn filename(&self, world: &World) -> Option<String> {
        Some(world.resource::<ProjectName>().as_filename())
    }
    fn on_load_without_filename(&self, _world: &mut World) {
        warn!("Load dialog was closed without file selected");
    }
    fn read(&self, world: &mut World, buffer: Vec<u8>) {
        // Load all data into memory before applying it to the world
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&buffer)).unwrap();

        let meta = Meta::load_from_zip(&mut zip).unwrap();
        let chunk_coords_and_data = ChunkCoordsAndData::load_from_zip(&mut zip).unwrap();
        let spawnables = Spawnables::load_from_zip(&mut zip).unwrap();

        // Apply it
        meta.apply_to_world(world);
        chunk_coords_and_data.apply_to_world(world);
        spawnables.apply_to_world(world);

        info!("Successfully loaded project");
    }
}

#[derive(Event)]
pub struct RequestSave;
#[derive(Clone, Copy)]
struct WorldSaveHandler;
impl file_picker::WriteHandler for WorldSaveHandler {
    type Event = RequestSave;

    fn filename(&self, world: &World) -> Option<String> {
        Some(world.resource::<ProjectName>().as_filename())
    }
    fn on_save_without_filename(&self, _world: &mut World) {
        warn!("Save dialog was closed without file selected");
    }
    fn write(&self, world: &mut World) -> Vec<u8> {
        let mut buffer = vec![];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        Meta::save_to_zip(&mut zip, world, options).unwrap();
        ChunkCoordsAndData::save_to_zip(&mut zip, world, options).unwrap();
        Spawnables::save_to_zip(&mut zip, world, options).unwrap();

        zip.finish().unwrap();
        buffer
    }
    fn on_write_complete(&self, _world: &mut World, result: std::io::Result<()>) {
        result.unwrap();
        info!("Successfully saved project");
    }
}
