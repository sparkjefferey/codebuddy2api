/// CC Switch deeplink — mirrors ccswitch.py
use urlencoding::encode;

pub fn build_deeplink(
    endpoint: &str,
    name: &str,
    api_key: &str,
    model: Option<&str>,
) -> String {
    let mut params: Vec<(String, String)> = vec![
        ("resource".into(), "provider".into()),
        ("app".into(), "claude".into()),
        ("name".into(), name.into()),
        ("homepage".into(), endpoint.into()),
        ("endpoint".into(), endpoint.into()),
        ("apiKey".into(), api_key.into()),
    ];
    if let Some(m) = model {
        if !m.is_empty() {
            params.insert(2, ("model".into(), m.into()));
        }
    }
    params.push(("configFormat".into(), "json".into()));
    params.push(("usageEnabled".into(), "false".into()));

    let qs: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("ccswitch://v1/import?{qs}")
}

pub fn open_deeplink(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        false
    }
}
