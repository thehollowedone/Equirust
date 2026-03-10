use std::sync::OnceLock;

const PRELUDE_SECTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/runtime/bootstrap/prelude.js"
));
const MEDIA_SECTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/runtime/bootstrap/media.js"
));
const INTEGRATIONS_SECTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/runtime/bootstrap/integrations.js"
));
const SETTINGS_SECTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/runtime/bootstrap/settings.js"
));

const TOKENS: &[&str] = &[
    "__EQUIRUST_SEED_JSON__",
    "__EQUIRUST_VENCORD_RENDERER_JSON__",
    "__EQUIRUST_CONTROL_RUNTIME_JSON__",
    "__EQUIRUST_INSTALL_HOST_RUNTIME_JSON__",
    "__EQUIRUST_INSTALL_MOD_RUNTIME_JSON__",
    "__EQUIRUST_SPOOF_EDGE_CLIENT_HINTS_JSON__",
];

fn bootstrap_template() -> &'static str {
    static TEMPLATE: OnceLock<String> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        [
            PRELUDE_SECTION,
            MEDIA_SECTION,
            INTEGRATIONS_SECTION,
            SETTINGS_SECTION,
        ]
        .concat()
    })
}

pub fn render_bootstrap_template(replacements: &[(&str, &str)]) -> Result<String, String> {
    let mut rendered = bootstrap_template().to_owned();
    for (token, value) in replacements {
        rendered = rendered.replace(token, value);
    }

    let unresolved = TOKENS
        .iter()
        .copied()
        .filter(|token| rendered.contains(token))
        .collect::<Vec<_>>();
    if !unresolved.is_empty() {
        return Err(format!(
            "desktop bootstrap template still contains unresolved tokens: {}",
            unresolved.join(", ")
        ));
    }

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::render_bootstrap_template;

    #[test]
    fn bootstrap_template_renders_without_unresolved_tokens() {
        let rendered = render_bootstrap_template(&[
            ("__EQUIRUST_SEED_JSON__", r#"{"debugBuild":true}"#),
            ("__EQUIRUST_VENCORD_RENDERER_JSON__", r#""""#),
            ("__EQUIRUST_CONTROL_RUNTIME_JSON__", "false"),
            ("__EQUIRUST_INSTALL_HOST_RUNTIME_JSON__", "true"),
            ("__EQUIRUST_INSTALL_MOD_RUNTIME_JSON__", "true"),
            ("__EQUIRUST_SPOOF_EDGE_CLIENT_HINTS_JSON__", "false"),
        ])
        .expect("bootstrap template should render");

        assert!(rendered.starts_with("(() => {"));
        assert!(rendered.contains(r#"const seed = {"debugBuild":true};"#));
        assert!(rendered.contains("const installModRuntime = true;"));
        for token in super::TOKENS {
            assert!(
                !rendered.contains(token),
                "rendered bootstrap still contains unresolved token {token}"
            );
        }
    }
}
