pub const DEFAULT_SECRET: &str = "future-proxy-secret";
pub const API_HOST: &str = "127.0.0.1:9090";
pub const MIXED_PORT: u16 = 7890;
pub const PATCH_BEGIN: &str = "# === Future Proxy Auto Injected Config ===";
pub const PATCH_END: &str = "# === Future Proxy Injected End ===";

/// 解析 Clash 标准 `Subscription-Userinfo: upload=123; download=456; total=789; expire=1825306223` 响应头
pub fn parse_subscription_userinfo(header: &str) -> (u64, u64, u64, u64) {
    let mut upload = 0u64;
    let mut download = 0u64;
    let mut total = 0u64;
    let mut expire = 0u64;
    for part in header.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            let key = k.trim().to_ascii_lowercase();
            let val_str = v.trim().trim_matches(|c| c == '"' || c == '\'');
            let val = val_str.parse::<u64>().unwrap_or(0);
            match key.as_str() {
                "upload" => upload = val,
                "download" => download = val,
                "total" => total = val,
                "expire" => expire = val,
                _ => {}
            }
        }
    }
    (upload, download, total, expire)
}

/// 从订阅 YAML 文本中的机场提示节点（如 `剩余流量：31.59 GB`、`套餐到期：长期有效`）提取兜底流量与到期信息
pub fn extract_yaml_traffic_hints(yaml: &str) -> (String, String) {
    let mut rem = String::new();
    let mut exp = String::new();
    let re_rem = regex::Regex::new(r"(?i)(?:剩余流量|剩餘流量|剩余|剩餘)\s*[:：]\s*([0-9.]+\s*[KMGTPE]?i?B)").unwrap();
    let re_exp = regex::Regex::new(r"(?i)(?:套餐到期|到期时间|到期|有效期)\s*[:：]\s*([^,'\}\]\r\n]+)").unwrap();

    for line in yaml.lines().take(300) {
        if rem.is_empty() {
            if let Some(c) = re_rem.captures(line) {
                rem = c[1].trim().trim_matches(|ch| ch == '\'' || ch == '"').to_string();
            }
        }
        if exp.is_empty() {
            if let Some(c) = re_exp.captures(line) {
                exp = c[1].trim().trim_matches(|ch| ch == '\'' || ch == '"').to_string();
            }
        }
        if !rem.is_empty() && !exp.is_empty() {
            break;
        }
    }
    (rem, exp)
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec < 1024 {
        format!("{} B/s", bytes_per_sec)
    } else if bytes_per_sec < 1024 * 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_unix_date(ts: u64) -> String {
    if ts == 0 {
        return String::new();
    }
    let days = ((ts + 8 * 3600) / 86400) as i64;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", year, m, d)
}

pub fn days_until_expire(ts: u64) -> Option<u64> {
    if ts == 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if ts <= now {
        Some(0)
    } else {
        Some((ts - now) / 86400)
    }
}

pub fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + 8 * 3600;
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400);
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:02}-{:02} {:02}:{:02}", m, d, hour, min)
}

pub fn detect_intranet_ip() -> String {
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("223.5.5.5:53").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

/// 判断是否为机场用于展示流量/到期/官网公告的假节点
pub fn is_info_node(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "剩余", "剩餘", "已用", "流量", "到期", "过期", "過期", "有效期", "重置",
        "官网", "官網", "网址", "網址", "域名", "公告", "套餐", "客服", "群组",
        "群組", "频道", "頻道", "通知", "订阅", "备用网址", "traffic", "remaining",
        "expire", "expiry", "reset", "website", "telegram",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

pub fn match_flag(name: &str) -> &'static str {
    if name.contains("非港台") {
        return "AUTO";
    }
    if name.contains("直接连接") || name.eq_ignore_ascii_case("direct") {
        return "DIR";
    }
    if name.contains("自动选择") || name.eq_ignore_ascii_case("auto") {
        return "AUTO";
    }
    if name.contains("故障转移") || name.eq_ignore_ascii_case("fallback") {
        return "FAIL";
    }
    if name.contains("🇦🇱") || is_info_node(name) {
        return "AL";
    }
    let lower = name.to_lowercase();
    if lower.contains("港") || lower.contains("hk") || lower.contains("hong kong") || name.contains("🇭🇰") {
        "HK"
    } else if lower.contains("台") || lower.contains("tw") || lower.contains("taiwan") || name.contains("🇹🇼") {
        "TW"
    } else if lower.contains("日") || lower.contains("jp") || lower.contains("japan") || name.contains("🇯🇵") {
        "JP"
    } else if lower.contains("美") || lower.contains("us") || lower.contains("united states") || name.contains("🇺🇸") {
        "US"
    } else if lower.contains("新") || lower.contains("sg") || lower.contains("singapore") || lower.contains("狮城") || name.contains("🇸🇬") {
        "SG"
    } else if lower.contains("韩") || lower.contains("kr") || lower.contains("korea") || name.contains("🇰🇷") {
        "KR"
    } else if lower.contains("英") || lower.contains("uk") || lower.contains("britain") || lower.contains("united kingdom") || name.contains("🇬🇧") {
        "UK"
    } else if lower.contains("德") || lower.contains("de") || lower.contains("germany") || name.contains("🇩🇪") {
        "DE"
    } else if lower.contains("荷") || lower.contains("nl") || lower.contains("netherlands") || name.contains("🇳🇱") {
        "NL"
    } else if lower.contains("马") || lower.contains("my") || lower.contains("malaysia") || name.contains("🇲🇾") {
        "MY"
    } else if lower.contains("瑞") || lower.contains("ch") || lower.contains("swiss") || name.contains("🇨🇭") {
        "CH"
    } else if lower.contains("法") || lower.contains("fr") || lower.contains("france") || name.contains("🇫🇷") {
        "FR"
    } else if lower.contains("加") || lower.contains("ca") || lower.contains("canada") || name.contains("🇨🇦") {
        "CA"
    } else if lower.contains("澳") || lower.contains("au") || lower.contains("australia") || name.contains("🇦🇺") {
        "AU"
    } else {
        "NET"
    }
}

pub fn region_match(region: &str, name: &str) -> bool {
    let lower = name.to_lowercase();
    let is = |a: &str, b: &str| lower.contains(a) || lower.contains(b);
    if region == "全部" {
        true
    } else if region.contains("非港台") {
        if name.contains("非港台") {
            return true;
        }
        let is_hk = lower.contains("港") || lower.contains("hk") || lower.contains("hong kong");
        let is_tw = lower.contains("台") || lower.contains("tw") || lower.contains("taiwan");
        let is_mo = lower.contains("澳") || lower.contains("macau") || lower.contains("macao");
        let is_cn = lower.contains("中国") || lower.contains("国内") || lower.contains("china");
        let is_smart_group = name.starts_with("⚡") || name == "自动选择" || name == "故障转移" || name == "🚀直接连接";
        !is_hk && !is_tw && !is_mo && !is_cn && !is_smart_group
    } else if region.contains("香港") {
        is("港", "hk")
    } else if region.contains("日本") {
        is("日", "jp")
    } else if region.contains("美国") {
        is("美", "us")
    } else if region.contains("新加坡") {
        is("新", "sg")
    } else if region.contains("台湾") {
        is("台", "tw")
    } else {
        !(is("港", "hk") || is("日", "jp") || is("美", "us") || is("新", "sg") || is("台", "tw") || name.contains("非港台"))
    }
}

static RE_MULT_SUFFIX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static RE_MULT_ANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

/// 解析节点名称中的倍率标签（如 "香港—F2 3倍率" -> ("香港—F2", Some("3x"))）
#[allow(dead_code)]
pub fn parse_node_display(name: &str) -> (String, Option<String>) {
    let re_suffix = RE_MULT_SUFFIX.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:[\[(（\s_-]*|\s+)(\d+(?:\.\d+)?)\s*(?:倍率|倍|x|X)(?:[\])）\s_-]*|\s*)$").unwrap()
    });
    if let Some(caps) = re_suffix.captures(name) {
        if let Some(m) = caps.get(1) {
            let mult = format!("{}x", m.as_str());
            let clean = re_suffix.replace(name, "").trim().to_string();
            return (clean, Some(mult));
        }
    }
    let re_any = RE_MULT_ANY.get_or_init(|| {
        regex::Regex::new(r"(?i)\s*(\d+(?:\.\d+)?)\s*(?:倍率|倍)\s*").unwrap()
    });
    if let Some(caps) = re_any.captures(name) {
        if let Some(m) = caps.get(1) {
            let mult = format!("{}x", m.as_str());
            let clean = re_any.replace(name, " ").trim().to_string();
            return (clean, Some(mult));
        }
    }
    (name.to_string(), None)
}

/// 格式化节点底层协议名称
pub fn format_proto_name(raw_type: &str) -> String {
    match raw_type.to_lowercase().as_str() {
        "vless" => "Vless".to_string(),
        "vmess" => "VMess".to_string(),
        "trojan" => "Trojan".to_string(),
        "ss" | "shadowsocks" => "SS".to_string(),
        "ssr" | "shadowsocksr" => "SSR".to_string(),
        "hysteria" => "Hysteria".to_string(),
        "hysteria2" | "hy2" => "Hysteria2".to_string(),
        "tuic" => "Tuic".to_string(),
        "wireguard" | "wg" => "WireGuard".to_string(),
        "urltest" | "url-test" => "URLTest".to_string(),
        "fallback" => "Fallback".to_string(),
        "loadbalance" | "load-balance" => "LoadBalance".to_string(),
        "selector" | "select" => "Select".to_string(),
        "direct" => "Direct".to_string(),
        other if !other.is_empty() => {
            let mut c = other.chars();
            match c.next() {
                None => "Proxy".to_string(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }
        _ => "Proxy".to_string(),
    }
}

pub fn looks_like_clash_config(body: &str) -> bool {
    body.lines().any(|l| l.starts_with("proxies:") || l.starts_with("proxy-providers:"))
}

const SMART_GROUPS_DEF: &str = r#"  - name: "⚡ 🌍 非港台-自动优选"
    type: url-test
    url: "https://www.gstatic.com/generate_204"
    interval: 180
    timeout: 2000
    tolerance: 50
    lazy: true
    max-failed-times: 3
    include-all: true
    exclude-type: "direct|reject"
    exclude-filter: "(?i)香港|HK|Hong Kong|台湾|TW|Taiwan|澳门|MO|中国|CN|China|剩余|剩餘|已用|流量|到期|过期|過期|有效期|重置|官网|官網|网址|網址|域名|公告|套餐|客服|群组|群組|频道|頻道|通知|traffic|remaining|expir|reset|website|telegram"
  - name: "⚡ 🇸🇬 新加坡-自动最优"
    type: url-test
    url: "https://www.gstatic.com/generate_204"
    interval: 180
    timeout: 2000
    tolerance: 50
    lazy: true
    max-failed-times: 3
    include-all: true
    exclude-type: "direct|reject"
    filter: "(?i)新加坡|狮城|SG|Singapore"
    exclude-filter: "(?i)剩余|剩餘|已用|流量|到期|过期|過期|有效期|重置|官网|官網|网址|網址|公告|套餐|客服|通知|traffic|remaining|expir|reset|website"
  - name: "⚡ 🇯🇵 日本-自动最优"
    type: url-test
    url: "https://www.gstatic.com/generate_204"
    interval: 180
    timeout: 2000
    tolerance: 50
    lazy: true
    max-failed-times: 3
    include-all: true
    exclude-type: "direct|reject"
    filter: "(?i)日本|东京|大阪|埼玉|JP|Japan"
    exclude-filter: "(?i)剩余|剩餘|已用|流量|到期|过期|過期|有效期|重置|官网|官網|网址|網址|公告|套餐|客服|通知|traffic|remaining|expir|reset|website"
  - name: "⚡ 🇺🇸 美国-自动最优"
    type: url-test
    url: "https://www.gstatic.com/generate_204"
    interval: 180
    timeout: 2000
    tolerance: 50
    lazy: true
    max-failed-times: 3
    include-all: true
    exclude-type: "direct|reject"
    filter: "(?i)美国|波特兰|洛杉矶|西雅图|芝加哥|硅谷|圣何塞|US|United States"
    exclude-filter: "(?i)剩余|剩餘|已用|流量|到期|过期|過期|有效期|重置|官网|官網|网址|網址|公告|套餐|客服|通知|traffic|remaining|expir|reset|website"
  - name: "⚡ 🇭🇰 香港-自动最优"
    type: url-test
    url: "https://www.gstatic.com/generate_204"
    interval: 180
    timeout: 2000
    tolerance: 50
    lazy: true
    max-failed-times: 3
    include-all: true
    exclude-type: "direct|reject"
    filter: "(?i)香港|HK|Hong Kong"
    exclude-filter: "(?i)剩余|剩餘|已用|流量|到期|过期|過期|有效期|重置|官网|官網|网址|網址|公告|套餐|客服|通知|traffic|remaining|expir|reset|website""#;

pub fn inject_smart_groups(lines: &[String]) -> Vec<String> {
    let mut group_indent = 2usize;
    let mut in_pg = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "proxy-groups:" {
            in_pg = true;
            continue;
        }
        if in_pg {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('-') {
                group_indent = line.len() - line.trim_start().len();
            }
            break;
        }
    }

    let pad = " ".repeat(group_indent);
    let mut smart_block = String::new();
    for raw_line in SMART_GROUPS_DEF.lines() {
        let stripped = raw_line.strip_prefix("  ").unwrap_or(raw_line);
        if !smart_block.is_empty() {
            smart_block.push('\n');
        }
        smart_block.push_str(&pad);
        smart_block.push_str(stripped);
    }

    let mut out = Vec::with_capacity(lines.len() + 60);
    let mut inserted_defs = false;

    for line in lines {
        out.push(line.to_string());
        let trimmed = line.trim();
        if trimmed == "proxy-groups:" && !inserted_defs {
            out.push(smart_block.clone());
            inserted_defs = true;
        }
    }

    out
}

#[allow(dead_code)]
pub fn patch_subscription(raw_yaml: &str, tun: bool) -> String {
    patch_subscription_with_mode(raw_yaml, tun, "rule")
}

pub fn patch_subscription_with_mode(raw_yaml: &str, tun: bool, mode: &str) -> String {
    let mut has_native_dns = false;
    let mut in_raw_old_patch = false;
    let mut in_dns_block = false;
    let re_key = regex::Regex::new(r"^([a-zA-Z0-9_-]+)\s*:").unwrap();

    for line in raw_yaml.lines() {
        if line.starts_with(PATCH_BEGIN) {
            in_raw_old_patch = true;
            continue;
        }
        if in_raw_old_patch {
            if line.starts_with(PATCH_END) || line.starts_with("# ========") {
                in_raw_old_patch = false;
            }
            continue;
        }
        if let Some(caps) = re_key.captures(line) {
            in_dns_block = &caps[1] == "dns";
        } else if in_dns_block && line.trim_start().starts_with("nameserver:") {
            has_native_dns = true;
        }
    }

    let override_keys: &[&str] = &[
        "external-controller",
        "external-controller-unix",
        "external-controller-pipe",
        "secret",
        "external-ui",
        "tun",
        "log-level",
        "ipv6",
        "mode",
        "port",
        "socks-port",
        "mixed-port",
        "redir-port",
        "tproxy-port",
        "allow-lan",
        "bind-address",
        "sniffer",
        "profile",
        "unified-delay",
        "find-process-mode",
        "tcp-concurrent",
        "geodata-loader",
        "keep-alive-idle",
        "keep-alive-interval",
    ];

    let mut filtered = Vec::new();
    let mut skipping = false;
    let mut in_old_patch = false;
    let mut in_kept_dns = false;

    for line in raw_yaml.lines() {
        if line.starts_with(PATCH_BEGIN) {
            in_old_patch = true;
            continue;
        }
        if in_old_patch {
            if line.starts_with(PATCH_END) || line.starts_with("# ========") {
                in_old_patch = false;
            }
            continue;
        }
        if let Some(caps) = re_key.captures(line) {
            let k = &caps[1];
            skipping = override_keys.contains(&k) || (!has_native_dns && k == "dns");
            in_kept_dns = has_native_dns && k == "dns";
        }
        if !skipping {
            if in_kept_dns && line.trim_start().starts_with("ipv6:") {
                let indent_len = line.len() - line.trim_start().len();
                filtered.push(format!("{}ipv6: false", " ".repeat(indent_len)));
            } else {
                filtered.push(line.to_string());
            }
        }
    }

    let fallback_dns = if has_native_dns {
        String::new()
    } else {
        r#"
dns:
  enable: true
  ipv6: false
  prefer-h3: false
  use-hosts: true
  use-system-hosts: false
  respect-rules: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - "*.lan"
    - "*.local"
    - "*.direct"
    - "*.localhost"
    - "*.localdomain"
    - "*.home.arpa"
    - "+.pool.ntp.org"
    - "time.*.com"
    - "ntp.*.com"
    - "stun.*.*"
    - "*.msftconnecttest.com"
    - "*.msftncsi.com"
    - "connectivitycheck.gstatic.com"
    - "captive.apple.com"
    - "+.wechat.com"
    - "+.weixin.qq.com"
    - "+.qq.com"
    - "+.qpic.cn"
    - "+.qlogo.cn"
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://doh.pub/dns-query
    - https://dns.alidns.com/dns-query
  proxy-server-nameserver:
    - https://dns.alidns.com/dns-query
    - https://doh.pub/dns-query
  direct-nameserver:
    - https://doh.pub/dns-query
    - https://dns.alidns.com/dns-query"#
            .to_string()
    };

    let valid_mode = match mode {
        "global" | "direct" | "rule" => mode,
        _ => "rule",
    };

    let header = format!(
        r#"{begin}
mixed-port: {port}
allow-lan: false
bind-address: "*"
external-controller: {api}
secret: "{secret}"
external-ui: ui
log-level: warning
ipv6: false
mode: {mode}
unified-delay: true
tcp-concurrent: true
geodata-loader: memconservative
find-process-mode: off
keep-alive-idle: 15
keep-alive-interval: 30
profile:
  store-selected: true
  store-fake-ip: true

tun:
  enable: {tun}
  stack: gvisor
  device: Mihomo
  mtu: 9000
  strict-route: false
  auto-route: true
  auto-redirect: false
  auto-detect-interface: true
  dns-hijack:
    - "any:53"
    - "tcp://any:53"
  route-exclude-address:
    - "127.0.0.0/8"
    - "10.0.0.0/8"
    - "172.16.0.0/12"
    - "192.168.0.0/16"
    - "169.254.0.0/16"
    - "224.0.0.0/4"
    - "255.255.255.255/32"
    - "fc00::/7"
    - "fe80::/10"

sniffer:
  enable: true
  parse-pure-ip: true
  force-dns-mapping: true
  override-destination: true
  sniff:
    HTTP:
      ports: [80, 8080-8880]
      override-destination: true
    TLS:
      ports: [443, 8443]
    QUIC:
      ports: [443, 8443]
  skip-domain:
    - "Mijia Cloud"
    - "+.push.apple.com"
    - "+.apple.com"
    - "+.wechat.com"
    - "+.weixin.qq.com"
    - "+.qq.com"
    - "+.qpic.cn"
    - "+.qlogo.cn"
    - "+.stun.*.*"{dns_section}
{end}"#,
        begin = PATCH_BEGIN,
        end = PATCH_END,
        port = MIXED_PORT,
        api = API_HOST,
        secret = DEFAULT_SECRET,
        mode = valid_mode,
        tun = tun,
        dns_section = fallback_dns,
    );

    let injected = inject_smart_groups(&filtered);
    format!("{}\n{}\n", header, injected.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn test_region_match_non_hktw() {
        assert!(region_match("🌍 非港台", "⚡ 🌍 非港台-自动优选"));
        assert!(region_match("🌍 非港台", "新加坡1—F0"));
        assert!(region_match("🌍 非港台", "日本原生GMO—F1"));
        assert!(region_match("🌍 非港台", "美国1—F1"));
        assert!(!region_match("🌍 非港台", "香港—F1"));
        assert!(!region_match("🌍 非港台", "台湾1—F2 3倍率"));
        assert!(!region_match("🌍 非港台", "自动选择"));
    }

    #[test]
    fn test_parse_node_display() {
        let (n1, r1) = parse_node_display("香港—F2 3倍率");
        assert_eq!(n1, "香港—F2");
        assert_eq!(r1, Some("3x".to_string()));

        let (n2, r2) = parse_node_display("台湾1—F2 3倍率");
        assert_eq!(n2, "台湾1—F2");
        assert_eq!(r2, Some("3x".to_string()));

        let (n3, r3) = parse_node_display("新加坡1—F1");
        assert_eq!(n3, "新加坡1—F1");
        assert_eq!(r3, None);

        let (n4, r4) = parse_node_display("美国1—F2 1.5x");
        assert_eq!(n4, "美国1—F2");
        assert_eq!(r4, Some("1.5x".to_string()));
    }

    #[test]
    fn test_smart_groups_injection() {
        let raw = r#"proxies:
  - name: "香港 01"
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: test
  - name: "新加坡 01"
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: test
proxy-groups:
  - name: 🔰国外流量
    type: select
    proxies:
      - 自动选择
      - 香港 01
      - 新加坡 01
  - name: 自动选择
    type: url-test
    proxies:
      - 香港 01
      - 新加坡 01
rules:
  - MATCH,🔰国外流量
"#;
        let patched = patch_subscription(raw, false);
        assert!(patched.contains("⚡ 🌍 非港台-自动优选"));
        assert!(patched.contains("⚡ 🇸🇬 新加坡-自动最优"));
        assert!(!patched.contains("\nipv6: true"));
        assert!(patched.contains("\nipv6: false"));

        let tmp_path = std::env::temp_dir().join("test_fp_smart.yaml");
        let _ = std::fs::write(&tmp_path, &patched);
        let out = Command::new("/usr/bin/mihomo-core")
            .args(["-t", "-f", tmp_path.to_str().unwrap()])
            .output();
        if let Ok(res) = out {
            assert!(
                res.status.success(),
                "mihomo configuration syntax check failed: {}",
                String::from_utf8_lossy(&res.stderr)
            );
        }
        let _ = std::fs::remove_file(tmp_path);
    }

    #[test]
    fn test_dns_preservation() {
        let raw_with_dns = r#"dns:
  enable: true
  ipv6: true
  enhanced-mode: fake-ip
  respect-rules: true
  nameserver:
    - https://cloudflare-dns.com/dns-query#🔰国外流量
proxies:
  - name: "新加坡 01"
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: test
proxy-groups:
  - name: 🔰国外流量
    type: select
    proxies:
      - 新加坡 01
rules:
  - MATCH,🔰国外流量
"#;
        let patched = patch_subscription(raw_with_dns, false);
        assert!(patched.contains("https://cloudflare-dns.com/dns-query#🔰国外流量"));
        assert!(patched.contains("respect-rules: true"));
        assert!(!patched.contains("\n  ipv6: true"));
    }

    #[test]
    fn test_zero_indent_and_inline_proxy_groups() {
        let zero_indent_raw = r#"proxies:
- name: "Japan 02"
  type: vless
  server: 1.1.1.1
  port: 443
  uuid: 00000000-0000-0000-0000-000000000000
proxy-groups:
- name: 节点选择
  type: select
  proxies:
  - DIRECT
  - Japan 02
rules:
- MATCH,节点选择
"#;
        let patched = patch_subscription(zero_indent_raw, false);
        assert!(patched.contains("\n- name: \"⚡ 🌍 非港台-自动优选\""));
        let tmp_path = std::env::temp_dir().join("test_fp_zero_indent.yaml");
        let _ = std::fs::write(&tmp_path, &patched);
        let out = Command::new("/usr/bin/future-proxy-core")
            .args(["-t", "-f", tmp_path.to_str().unwrap()])
            .output();
        if let Ok(res) = out {
            assert!(
                res.status.success(),
                "future-proxy-core syntax check failed on zero-indent YAML: {}",
                String::from_utf8_lossy(&res.stdout)
            );
        }
        let _ = std::fs::remove_file(tmp_path);
    }

    #[test]
    fn test_real_profiles_and_regenerate_active_config() {
        let cfg_dir = PathBuf::from("/home/hu-hub/.config/future-proxy");
        let profiles_dir = cfg_dir.join("profiles");
        if profiles_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                        let raw = std::fs::read_to_string(&path).unwrap();
                        let patched = patch_subscription_with_mode(&raw, false, "rule");
                        let tmp = std::env::temp_dir().join(format!(
                            "test_fp_{}",
                            path.file_name().unwrap().to_string_lossy()
                        ));
                        std::fs::write(&tmp, &patched).unwrap();
                        let out = Command::new("/usr/bin/future-proxy-core")
                            .args(["-t", "-f", tmp.to_str().unwrap()])
                            .output()
                            .unwrap();
                        let _ = std::fs::remove_file(&tmp);
                        assert!(
                            out.status.success(),
                            "Profile {:?} failed syntax check: {}",
                            path,
                            String::from_utf8_lossy(&out.stdout)
                        );
                    }
                }
            }
            let active_sub = profiles_dir.join("sub_1793770259.yaml");
            if active_sub.exists() {
                let active_cfg_path = cfg_dir.join("config.yaml");
                let cur_tun = std::fs::read_to_string(&active_cfg_path)
                    .map(|c| c.contains("tun:\n  enable: true"))
                    .unwrap_or(false);
                let raw = std::fs::read_to_string(&active_sub).unwrap();
                let patched = patch_subscription_with_mode(&raw, cur_tun, "rule");
                std::fs::write(active_cfg_path, &patched).unwrap();
            }
        }
    }

    #[test]
    fn test_subscription_userinfo_and_hints() {
        let (up, down, tot, exp) = parse_subscription_userinfo(
            "upload=6928073; download=93802546; total=137438953472; expire=1825306223",
        );
        assert_eq!(up, 6928073);
        assert_eq!(down, 93802546);
        assert_eq!(tot, 137438953472);
        assert_eq!(exp, 1825306223);
        assert_eq!(format_bytes(up + down), "96.06 MB");
        assert_eq!(format_bytes(tot - (up + down)), "127.91 GB");
        assert_eq!(format_bytes(tot), "128.00 GB");
        assert_eq!(format_unix_date(exp), "2027-11-04");

        let (rem, exp_h) = extract_yaml_traffic_hints(
            "proxies:\n  - name: 🇦🇱 剩余流量：127.55 GB\n  - name: 🇦🇱 套餐到期：2027-11-04\n",
        );
        assert_eq!(rem, "127.55 GB");
        assert_eq!(exp_h, "2027-11-04");
    }
}
