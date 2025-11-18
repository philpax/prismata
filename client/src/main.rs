#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use bevy::{asset::AssetMetaCheck, prelude::*};

use avian3d::prelude::*;

mod camera;
mod color;
mod config;
mod file_picker;
mod load_save;
mod picking;
mod play_mode;
mod raycast;
mod rendering;
mod tools;
mod ui;
mod util;
mod voxel;

#[derive(Resource)]
/// The address of the HTTP server used for authentication and other out-of-band requests.
pub struct HttpEndpoints {
    pub render_url: String,
}
impl HttpEndpoints {
    pub fn from_server(server: &str) -> Self {
        Self {
            render_url: format!("{server}/prism/render"),
        }
    }
}

#[derive(Resource, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ProjectName(pub String);
impl Default for ProjectName {
    fn default() -> Self {
        Self("New Project".to_string())
    }
}
impl ProjectName {
    pub fn as_filename(&self) -> String {
        let base_name = self
            .0
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();

        format!("{base_name}.prismata")
    }
}

#[derive(States, Default, Clone, Eq, PartialEq, Debug, Hash)]
pub enum AppState {
    #[default]
    Edit,
    Play,
}
impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

fn main() {
    let mut app = App::new();
    // Add before DefaultPlugins to override the default asset loading.
    app.add_plugins(bevy_embedded_assets::EmbeddedAssetPlugin {
        mode: bevy_embedded_assets::PluginMode::ReplaceDefault,
    });
    init_app(&mut app);

    app.insert_resource(ProjectName::default())
        .init_state::<AppState>()
        .add_plugins((
            bevy_mod_reqwest::ReqwestPlugin::default(),
            transform_gizmo_bevy::TransformGizmoPlugin,
            bevy_egui::EguiPlugin::default(),
            avian3d::PhysicsPlugins::default(),
            avian3d::debug_render::PhysicsDebugPlugin::default(),
            picking::plugin,
            raycast::plugin,
            load_save::plugin,
            camera::plugin,
            rendering::plugin,
            voxel::plugin,
            tools::plugin,
            ui::plugin,
            play_mode::plugin,
        ))
        .insert_resource(transform_gizmo_bevy::GizmoOptions {
            hotkeys: Some(transform_gizmo_bevy::GizmoHotkeys::default()),
            ..default()
        })
        .add_systems(Startup, |mut store: ResMut<GizmoConfigStore>| {
            store.config_mut::<PhysicsGizmos>().0.enabled = false;
        })
        .add_systems(Update, swap_state)
        .run();
}

#[cfg(feature = "native")]
fn init_app(app: &mut App) {
    use std::str::FromStr;

    use clap::Parser;

    #[derive(Clone, Debug)]
    enum Backend {
        Host,
        Server(String),
    }
    impl FromStr for Backend {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "host" => Ok(Self::Host),
                s if s.starts_with("http") => Ok(Self::Server(s.to_string())),
                _ => Err(format!("Invalid backend: {}", s)),
            }
        }
    }

    #[derive(Parser)]
    #[command(author, version, about, long_about = None)]
    struct Cli {
        #[arg(long)]
        backend: Backend,
        #[arg(short, long, num_args = 2)]
        /// The position of the window.
        position: Option<Vec<u32>>,
        #[arg(short, long, num_args = 2)]
        /// The size of the window.
        size: Option<Vec<u32>>,
    }
    let cli = Cli::parse();

    #[derive(Resource)]
    struct ServerThread {
        pub _thread: Option<std::thread::JoinHandle<()>>,
    }

    let (http_endpoints, server_thread) = match cli.backend {
        Backend::Server(server) => (HttpEndpoints::from_server(&server), None),
        Backend::Host => {
            let client_server_settings = config::Config::read()
                .server
                .clone()
                .expect("Listen server requested, but no server settings found in config");
            let server_thread = std::thread::spawn(move || {
                tokio::runtime::Runtime::new().unwrap().block_on(async {
                    prismata_server_lib::main(&prismata_server_lib::Settings {
                        logger: false,
                        comfyui_address: client_server_settings.comfyui_address,
                        comfyui_authentication: client_server_settings.comfyui_authentication,
                        comfyui_https_insecure: client_server_settings.comfyui_https_insecure,
                        workflow: client_server_settings.workflow_settings,
                        ..default()
                    })
                    .await;
                });
            });

            (
                HttpEndpoints::from_server(&format!(
                    "http://127.0.0.1:{}",
                    prismata_protocol::DEFAULT_HTTP_SERVER_PORT
                )),
                Some(server_thread),
            )
        }
    };

    let config_size = config::Config::read().size();
    let arg_size = cli.size.map(|v| (v[0], v[1]));
    let size = arg_size.unwrap_or(config_size);

    let position = cli
        .position
        .map(|v| IVec2::new(v[0] as i32, v[1] as i32))
        .map(WindowPosition::new)
        .unwrap_or_default();

    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                meta_check: AssetMetaCheck::Never,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Prismata".to_string(),
                    position,
                    resolution: bevy::window::WindowResolution::new(size.0 as f32, size.1 as f32),
                    ..default()
                }),
                ..default()
            }),
    )
    .insert_resource(ServerThread {
        _thread: server_thread,
    })
    .insert_resource(http_endpoints);
}

#[cfg(not(feature = "native"))]
fn init_app(app: &mut App) {
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                meta_check: AssetMetaCheck::Never,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Prismata".to_string(),
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            }),
    );
    // Disable HTTP endpoints in Web builds
    // .insert_resource(HttpEndpoints::from_function());
}

fn swap_state(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<AppState>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::F5) {
        match state.get() {
            AppState::Edit => next_state.set(AppState::Play),
            AppState::Play => next_state.set(AppState::Edit),
        }
    }
}
