//! Publishing the page to the tailnet with `tailscale serve`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// HTTPS ports tried in order when none is given. 443 gives an address without a port.
pub const AUTO_PORTS: &[u16] = &[443, 8443, 10000];

fn cli() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("tailscale"));
    // The Mac app keeps its command-line tool inside the app bundle.
    let bundled = [PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale")];
    on_path.chain(bundled).find(|path| path.is_file())
}

fn read_json(cli: &PathBuf, args: &[&str]) -> Result<Value, String> {
    let output = Command::new(cli).args(args).output().map_err(|e| e.to_string())?;
    serde_json::from_slice(&output.stdout).map_err(|e| format!("tailscale {} gave no JSON ({e})", args.join(" ")))
}

fn same_target(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// What this device already publishes, by port, from `tailscale serve status --json`.
fn published(config: &Value, dns_name: &str) -> BTreeMap<u16, String> {
    let mut ports = BTreeMap::new();
    for (port, entry) in config["TCP"].as_object().into_iter().flatten() {
        let Ok(port) = port.parse::<u16>() else { continue };
        let what = match entry["TCPForward"].as_str() {
            Some(forward) => format!("a TCP forward to {forward}"),
            None => "an HTTPS server".to_string(),
        };
        ports.insert(port, what);
    }
    let prefix = format!("{dns_name}:");
    for (host, site) in config["Web"].as_object().into_iter().flatten() {
        let Some(port) = host.strip_prefix(&prefix).and_then(|p| p.parse::<u16>().ok()) else {
            continue;
        };
        let handlers = site["Handlers"].as_object();
        // The root handler names the server; otherwise the first mount does.
        let root = handlers.and_then(|h| h.get("/"));
        let any = root.or_else(|| handlers.and_then(|h| h.values().next()));
        let what = any
            .and_then(|h| {
                h["Proxy"]
                    .as_str()
                    .or_else(|| h["Path"].as_str())
                    .or_else(|| h["Text"].as_str().map(|_| "a text reply"))
            })
            .unwrap_or("an HTTPS server");
        ports.insert(port, what.to_string());
    }
    ports
}

/// The HTTPS port to publish `target` on. A port that already publishes `target` is reused, so the
/// address stays the same; a port that publishes anything else is never taken over.
pub fn choose_port(config: &Value, dns_name: &str, target: &str, requested: Option<u16>) -> Result<u16, String> {
    let ports = published(config, dns_name);
    let ours = |port: &u16| ports.get(port).is_some_and(|what| same_target(what, target));
    if let Some(port) = requested {
        return match ports.get(&port) {
            Some(what) if !same_target(what, target) => {
                let free = AUTO_PORTS.iter().find(|p| !ports.contains_key(p));
                let hint = free.map(|p| format!(" Choose another, e.g. --tailscale {p}.")).unwrap_or_default();
                Err(format!("HTTPS port {port} already publishes {what} on this Mac.{hint}"))
            }
            _ => Ok(port),
        };
    }
    if let Some(port) = ports.keys().find(|p| ours(p)) {
        return Ok(*port);
    }
    AUTO_PORTS.iter().copied().find(|p| !ports.contains_key(p)).ok_or_else(|| {
        format!(
            "HTTPS ports {} all publish other servers on this Mac. Choose a free one with --tailscale <port>.",
            AUTO_PORTS.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
        )
    })
}

pub fn address(dns_name: &str, https_port: u16) -> String {
    if https_port == 443 {
        format!("https://{dns_name}")
    } else {
        format!("https://{dns_name}:{https_port}")
    }
}

/// Publish `target` to the tailnet over HTTPS, on `requested` or the first free port; returns the
/// address devices open and the port used. Other servers this device publishes are left alone.
///
/// `tailscale serve --bg` stores the setting in Tailscale itself, so it survives restarts of this
/// server and of the computer, and only answers while this server runs. Its output is not
/// captured: the first time, Tailscale may print a link to enable HTTPS on the tailnet.
pub fn serve(target: &str, requested: Option<u16>) -> Result<(String, u16), String> {
    let cli = cli().ok_or("The tailscale command was not found. Install Tailscale and sign in, or drop --tailscale.")?;
    let unreadable = |e: String| format!("Could not read Tailscale's status ({e}). Is Tailscale running and signed in?");
    let status = read_json(&cli, &["status", "--json"]).map_err(unreadable)?;
    let dns_name = status["Self"]["DNSName"]
        .as_str()
        .ok_or_else(|| unreadable("no DNS name".into()))?
        .trim_end_matches('.')
        .to_string();
    let before = read_json(&cli, &["serve", "status", "--json"]).unwrap_or(Value::Null);
    let https_port = choose_port(&before, &dns_name, target, requested)?;
    for (port, what) in published(&before, &dns_name) {
        if port != https_port {
            println!("  Keeping   https port {port} → {what} (another server)");
        }
    }

    let port_flag = format!("--https={https_port}");
    println!("  Running   {} serve --bg {port_flag} {target}", cli.display());
    let status = Command::new(&cli)
        .args(["serve", "--bg", &port_flag, target])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let hint = if cfg!(target_os = "linux") {
            " On Linux, allow your user once with: sudo tailscale set --operator=$USER"
        } else {
            ""
        };
        return Err(format!("tailscale serve failed (see its message above).{hint}"));
    }

    let config = read_json(&cli, &["serve", "status", "--json"]).map_err(unreadable)?;
    // When HTTPS is not yet enabled on the tailnet, serve prints a link and exits 0 without publishing.
    if !published(&config, &dns_name)
        .get(&https_port)
        .is_some_and(|what| same_target(what, target))
    {
        return Err("Tailscale did not publish the page. If it printed a link above, open it to enable HTTPS for your tailnet, then start this again.".into());
    }
    Ok((address(&dns_name, https_port), https_port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DNS: &str = "my-mac.tail1a2b3c.ts.net";
    const US: &str = "http://127.0.0.1:8189";

    /// This Mac today: Hermes WebUI on 443.
    fn hermes_on_443() -> Value {
        json!({
            "TCP": {"443": {"HTTPS": true}},
            "Web": {"my-mac.tail1a2b3c.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8787"}}}}
        })
    }

    #[test]
    fn nothing_published_takes_443() {
        assert_eq!(choose_port(&Value::Null, DNS, US, None), Ok(443));
        assert_eq!(choose_port(&json!({}), DNS, US, None), Ok(443));
    }

    #[test]
    fn another_server_on_443_moves_us_to_8443() {
        assert_eq!(choose_port(&hermes_on_443(), DNS, US, None), Ok(8443));
    }

    #[test]
    fn our_own_port_is_reused() {
        let mut config = hermes_on_443();
        config["TCP"]["10000"] = json!({"HTTPS": true});
        config["Web"]["my-mac.tail1a2b3c.ts.net:10000"] = json!({"Handlers": {"/": {"Proxy": "http://127.0.0.1:8189/"}}});
        assert_eq!(choose_port(&config, DNS, US, None), Ok(10000));
    }

    #[test]
    fn a_requested_port_held_by_another_server_is_refused() {
        let error = choose_port(&hermes_on_443(), DNS, US, Some(443)).unwrap_err();
        assert!(error.contains("http://127.0.0.1:8787"), "{error}");
        assert!(error.contains("--tailscale 8443"), "{error}");
        assert_eq!(choose_port(&hermes_on_443(), DNS, US, Some(9443)), Ok(9443));
    }

    #[test]
    fn tcp_forwards_and_path_mounts_count_as_taken() {
        let config = json!({
            "TCP": {"443": {"TCPForward": "127.0.0.1:22"}, "8443": {"HTTPS": true}},
            "Web": {"my-mac.tail1a2b3c.ts.net:8443": {"Handlers": {"/files": {"Path": "/Users/me/share"}}}}
        });
        assert_eq!(choose_port(&config, DNS, US, None), Ok(10000));
        let error = choose_port(&config, DNS, US, Some(8443)).unwrap_err();
        assert!(error.contains("/Users/me/share"), "{error}");
    }

    #[test]
    fn addresses_leave_out_443() {
        assert_eq!(address(DNS, 443), "https://my-mac.tail1a2b3c.ts.net");
        assert_eq!(address(DNS, 8443), "https://my-mac.tail1a2b3c.ts.net:8443");
    }
}
