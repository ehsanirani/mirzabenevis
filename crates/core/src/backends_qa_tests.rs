//! QA tests for backends.rs (from the review). `regression_*` tests pin bugs that
//! were found and fixed; their `BUG:` comments describe the original defect.
use super::*;

fn env(vars: &[(&str, &str)]) -> Env {
    Env::for_test(vars, vec![], &[], &[])
}

fn ollama() -> &'static Local {
    LOCALS.iter().find(|l| l.id == "ollama").unwrap()
}

fn api(id: &str) -> &'static Api {
    APIS.iter().find(|a| a.id == id).unwrap()
}

fn wire_body(id: &str) -> Value {
    let cfg = Config::default();
    let b = api_backend(api(id), &cfg, Secret("k".into()), "test".into());
    let gen = crate::provider::Gen::from(&cfg.provider);
    match &b.transport {
        Transport::OpenAi(w) => crate::provider::openai_body(w, &b.model, &gen, "S", "U"),
        t => panic!("{id}: not openai wire: {t:?}"),
    }
}

#[test]
fn catalog_request_bodies() {
    let ds = wire_body("deepseek");
    assert_eq!(ds["thinking"], serde_json::json!({"type": "disabled"}));
    assert!(ds.get("temperature").is_some());
    assert_eq!(ds["max_tokens"], 16384);
    assert_eq!(ds["model"], "deepseek-flash");

    let oa = wire_body("openai");
    assert!(oa.get("temperature").is_none());
    assert!(oa.get("max_tokens").is_none());
    assert_eq!(oa["max_completion_tokens"], 16384);
    assert_eq!(oa["reasoning_effort"], "none");

    let q = wire_body("qwen");
    assert_eq!(q["enable_thinking"], false);
}

#[test]
fn regression_ipv6_loopback_is_not_treated_as_local() {
    // BUG: host is split on ':' before the brackets are handled, so the host is "[".
    // A custom base_url on [::1] is then sent through HTTP(S)_PROXY.
    assert!(
        is_local_url("http://[::1]:8080/v1"),
        "[::1] should be local"
    );
}

#[test]
fn regression_ollama_host_port_only() {
    // BUG: OLLAMA_HOST=":11500" (accepted by Ollama) becomes "http://:11500".
    assert_eq!(
        local_bases(ollama(), &env(&[("OLLAMA_HOST", ":11500")])),
        ["http://127.0.0.1:11500"]
    );
}

#[test]
fn regression_ollama_host_with_path() {
    // BUG: a path after the port makes the port check fail, so ":11434" is appended
    // to the path: "http://box:11434/ollama:11434".
    assert_eq!(
        local_bases(
            ollama(),
            &env(&[("OLLAMA_HOST", "http://box:11434/ollama")])
        ),
        ["http://box:11434/ollama"]
    );
}

#[test]
fn regression_ollama_host_ipv6_any() {
    // BUG: "[::]:11434" (listen on all IPv6) is probed as-is: http://[::]:11434.
    assert_eq!(
        local_bases(ollama(), &env(&[("OLLAMA_HOST", "[::]:11434")])),
        ["http://[::1]:11434"]
    );
}

#[test]
fn regression_ollama_host_scheme_default_port() {
    // Ollama itself uses port 443 for "https://host" and 80 for "http://host";
    // here 11434 is appended instead (https://ollama.example.com:11434).
    assert_eq!(
        local_bases(
            ollama(),
            &env(&[("OLLAMA_HOST", "https://ollama.example.com")])
        ),
        ["https://ollama.example.com"]
    );
}

#[test]
fn regression_embedding_models_without_embed_in_name_are_picked() {
    // BUG: only names containing "embed" are skipped; bge-m3 / all-minilm are
    // embedding-only Ollama models and get chosen as the chat model.
    let tags = serde_json::json!({"models":[{"name":"bge-m3:latest"},{"name":"llama3.2:3b"}]});
    assert_eq!(
        parse_model_list(ModelList::OllamaTags, &tags),
        ["llama3.2:3b"]
    );
}

#[test]
fn regression_empty_env_var_hides_login_shell_value() {
    // BUG: `FOO=` in the process env keeps the login shell's non-empty FOO out of
    // shell_vars, so the key is not found at all.
    let mut vars = HashMap::new();
    vars.insert("DEEPSEEK_API_KEY".to_string(), String::new());
    let mut shell = HashMap::new();
    shell.insert("DEEPSEEK_API_KEY".to_string(), "sk-shell".to_string());
    let e = Env::build(vars, Some(shell), None);
    assert_eq!(e.var("DEEPSEEK_API_KEY"), Some(("sk-shell", true)));
}

#[test]
fn gemini_env_precedence_and_trimming() {
    let d = detect_with_locals(
        &Config::default(),
        &env(&[("GEMINI_API_KEY", " g1 \n"), ("GOOGLE_API_KEY", "g2")]),
        vec![],
    );
    let g = d.pick("gemini").unwrap();
    assert_eq!(g.source, "GEMINI_API_KEY (environment)");
    match &g.transport {
        Transport::OpenAi(w) => assert_eq!(w.api_key.as_ref().unwrap().0, "g1"),
        t => panic!("{t:?}"),
    }
}

#[test]
fn regression_unconfigured_custom_is_reported_as_unknown() {
    // `custom` is a known id (`config use custom` accepts it) but picking it while
    // [provider.custom] is empty says "unknown backend `custom`".
    let d = detect_with_locals(&Config::default(), &env(&[]), vec![]);
    let e = d.pick("custom").unwrap_err().to_string();
    assert!(!e.contains("unknown backend"), "{e}");
}

#[test]
fn regression_relative_custom_command_is_found_but_cannot_run() {
    // Detection checks a relative argv[0] against our cwd and keeps it relative,
    // but run_process starts the child in the temp dir, where it does not exist.
    // (cargo runs tests from crates/core, so this relative path exists.)
    let rel = "../../tests-e2e/fake_agent.sh";
    assert!(is_executable(Path::new(rel)));
    let mut cfg = Config::default();
    cfg.provider.custom.kind = ProviderKind::Command;
    cfg.provider.custom.command = vec![rel.into()];
    let d = detect_with_locals(&cfg, &env(&[]), vec![]);
    let b = d.pick("custom").expect("detected");
    match &b.transport {
        Transport::Command { argv, .. } => assert!(
            Path::new(&argv[0]).is_absolute(),
            "relative argv[0] {:?} breaks once cwd is the temp dir",
            argv[0]
        ),
        t => panic!("{t:?}"),
    }
}
