//! `--once` mode: one telemetry sample as compact JSON for scripts
//! (waybar & co). No serde — the schema is fixed and tiny.

use rusense_core::Telemetry;

/// Escape `"`, `\` and control characters for embedding in a JSON
/// string literal.
fn escape_json(s: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Render one sample as single-line JSON, temps with one decimal.
pub fn telemetry_json(t: &Telemetry) -> String {
    format!(
        concat!(
            "{{\"fan_cpu_rpm\":{},\"fan_gpu_rpm\":{},",
            "\"temp_cpu\":{:.1},\"temp_gpu\":{:.1},\"temp_sys\":{:.1},",
            "\"battery_pct\":{},\"battery_status\":\"{}\"}}"
        ),
        t.fan_cpu_rpm,
        t.fan_gpu_rpm,
        t.temps[0],
        t.temps[1],
        t.temps[2],
        t.battery_pct,
        escape_json(&t.battery_status)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(status: &str) -> Telemetry {
        Telemetry {
            fan_cpu_rpm: 2348,
            fan_gpu_rpm: 2081,
            temps: [41.0, 35.0, 40.0],
            battery_pct: 80,
            battery_status: status.to_string(),
        }
    }

    #[test]
    fn telemetry_json_is_compact_single_line() {
        assert_eq!(
            telemetry_json(&sample("Not charging")),
            "{\"fan_cpu_rpm\":2348,\"fan_gpu_rpm\":2081,\
             \"temp_cpu\":41.0,\"temp_gpu\":35.0,\"temp_sys\":40.0,\
             \"battery_pct\":80,\"battery_status\":\"Not charging\"}"
        );
    }

    #[test]
    fn telemetry_json_escapes_battery_status() {
        let json = telemetry_json(&sample("st\"atus\\x"));
        assert!(json.contains("\"battery_status\":\"st\\\"atus\\\\x\""));
    }

    #[test]
    fn escape_json_escapes_quotes_and_backslashes() {
        assert_eq!(escape_json("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(escape_json("Not charging"), "Not charging");
    }

    #[test]
    fn escape_json_escapes_control_characters() {
        assert_eq!(escape_json("a\nb\tc"), "a\\u000ab\\u0009c");
    }
}
