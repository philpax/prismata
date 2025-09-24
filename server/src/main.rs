use prismata_server_lib as server_lib;

mod config;

#[tokio::main]
async fn main() {
    let config = config::Config::read().clone();

    server_lib::main(&server_lib::Settings {
        http_port: config.http_port,
        logger: true,
        comfyui_address: config.comfyui_address,
        comfyui_authentication: config.comfyui_authentication,
        comfyui_https_insecure: config.comfyui_https_insecure,
        workflow: config.workflow_settings,
    })
    .await;
}
