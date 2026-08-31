/// Billing — credits & checkin, mirrors billing.py
use std::collections::HashMap;

const PRODUCT_CODE: &str = "p_tcaca";

pub async fn query_credits(headers: &HashMap<String, String>) -> serde_json::Value {
    let url = "https://www.codebuddy.cn/v2/billing/meter/get-user-resource";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Format dates as YYYY-MM-DD HH:MM:SS
    let begin = format_ts(now);
    let end = format_ts(now + 365 * 101 * 86400);

    let body = serde_json::json!({
        "PageNumber": 1,
        "PageSize": 100,
        "ProductCode": PRODUCT_CODE,
        "Status": [0, 3],
        "PackageEndTimeRangeBegin": begin,
        "PackageEndTimeRangeEnd": end,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"error": e.to_string()}),
    };

    let mut req = client.post(url).json(&body);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return serde_json::json!({"error": e.to_string()}),
    };
    if resp.status() != reqwest::StatusCode::OK {
        return serde_json::json!({"error": format!("HTTP {}", resp.status())});
    }
    let data: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"error": e.to_string()}),
    };
    if data.get("code").and_then(|v| v.as_i64()) != Some(0) {
        return serde_json::json!({"error": format!("code={} msg={}", data.get("code").and_then(|v| v.as_i64()).unwrap_or(-1), data.get("msg").and_then(|v| v.as_str()).unwrap_or(""))});
    }
    let accounts = data
        .get("data")
        .and_then(|v| v.get("Response"))
        .and_then(|v| v.get("Data"))
        .and_then(|v| v.get("Accounts"))
        .and_then(|v| v.as_array());

    let Some(accounts) = accounts else {
        return serde_json::json!({"credits_remaining": 0, "packages": []});
    };

    let mut remain: i64 = 0;
    let mut raw: Vec<serde_json::Value> = Vec::new();
    for a in accounts {
        let cap_remain = a.get("CapacityRemain").and_then(|v| v.as_i64()).unwrap_or(0);
        let cycle_size = a.get("CycleCapacitySize").and_then(|v| v.as_i64()).unwrap_or(0);
        let cycle_remain = a.get("CycleCapacityRemain").and_then(|v| v.as_i64()).unwrap_or(0);
        let cycle_used = a.get("CycleCapacityUsed").and_then(|v| v.as_i64()).unwrap_or(0);
        let r_val = if cycle_remain > 0 || cycle_used > 0 || cycle_size > 0 {
            cycle_remain
        } else {
            cap_remain
        };
        remain += r_val.max(0);
        raw.push(serde_json::json!({
            "package": a.get("PackageName"),
            "capacity_remain": a.get("CapacityRemain"),
            "cycle_capacity_remain": a.get("CycleCapacityRemain"),
        }));
    }
    serde_json::json!({"credits_remaining": remain, "packages": raw})
}

pub async fn daily_checkin(headers: &HashMap<String, String>) -> serde_json::Value {
    let url = "https://www.codebuddy.cn/v2/billing/meter/daily-checkin";
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"ok": false, "error": e.to_string()}),
    };
    let mut req = client.post(url).json(&serde_json::json!({}));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return serde_json::json!({"ok": false, "error": e.to_string()}),
    };
    let status = resp.status();
    let data: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => {
            return serde_json::json!({"ok": false, "error": format!("HTTP {}", status)})
        }
    };
    let code = data.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code == 0 {
        serde_json::json!({"ok": true, "message": "签到成功"})
    } else {
        let msg = data
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("已签到(code={code})"))
            .to_string();
        serde_json::json!({"ok": false, "message": msg, "code": code})
    }
}

fn format_ts(secs: u64) -> String {
    // civil_from_days algorithm for UTC
    let days = (secs / 86400) as i64;
    let secs_of_day = (secs % 86400) as u32;
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}
