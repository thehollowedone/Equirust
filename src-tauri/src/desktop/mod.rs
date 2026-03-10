mod template;

use serde_json::Value;

pub fn bootstrap_script(
    seed: &Value,
    vencord_renderer: Option<&str>,
    control_runtime: bool,
    install_host_runtime: bool,
    install_mod_runtime: bool,
    spoof_edge_client_hints: bool,
) -> Result<String, String> {
    let seed_json = serde_json::to_string(seed).map_err(|err| err.to_string())?;
    let vencord_renderer_json =
        serde_json::to_string(&vencord_renderer.unwrap_or("")).map_err(|err| err.to_string())?;
    let control_runtime_json =
        serde_json::to_string(&control_runtime).map_err(|err| err.to_string())?;
    let install_host_runtime_json =
        serde_json::to_string(&install_host_runtime).map_err(|err| err.to_string())?;
    let install_mod_runtime_json =
        serde_json::to_string(&install_mod_runtime).map_err(|err| err.to_string())?;
    let spoof_edge_client_hints_json =
        serde_json::to_string(&spoof_edge_client_hints).map_err(|err| err.to_string())?;

    template::render_bootstrap_template(&[
        ("__EQUIRUST_SEED_JSON__", seed_json.as_str()),
        (
            "__EQUIRUST_VENCORD_RENDERER_JSON__",
            vencord_renderer_json.as_str(),
        ),
        (
            "__EQUIRUST_CONTROL_RUNTIME_JSON__",
            control_runtime_json.as_str(),
        ),
        (
            "__EQUIRUST_INSTALL_HOST_RUNTIME_JSON__",
            install_host_runtime_json.as_str(),
        ),
        (
            "__EQUIRUST_INSTALL_MOD_RUNTIME_JSON__",
            install_mod_runtime_json.as_str(),
        ),
        (
            "__EQUIRUST_SPOOF_EDGE_CLIENT_HINTS_JSON__",
            spoof_edge_client_hints_json.as_str(),
        ),
    ])
}
