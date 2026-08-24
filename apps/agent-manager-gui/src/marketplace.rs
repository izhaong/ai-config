//! [Claude Code Marketplace](https://www.claudemarketplace.net/skills) 列表 API 代理。
//!
//! Tauri WebView 无法直接跨域 fetch，故由 Rust 侧请求 `GET /api/skills`。

use std::time::Duration;

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://www.claudemarketplace.net/api/skills";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    pub slug: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub github_stars: u64,
    #[serde(default)]
    pub installs: u64,
    pub github_language: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MarketplaceListResult {
    pub skills: Vec<MarketplaceSkill>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Deserialize)]
struct ApiResponse {
    skills: Vec<MarketplaceSkill>,
    total: u64,
}

pub fn list_skills(
    sort: &str,
    q: Option<&str>,
    source_filter: Option<&str>,
    offset: u32,
    limit: u32,
) -> Result<MarketplaceListResult, String> {
    let sort = match sort {
        "stars" | "newest" | "installs" => sort,
        _ => "installs",
    };
    let limit = limit.clamp(1, 100);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;

    let mut req = client.get(API_BASE).query(&[
        ("sort", sort),
        ("offset", &offset.to_string()),
        ("limit", &limit.to_string()),
    ]);

    if let Some(query) = q.filter(|s| !s.trim().is_empty()) {
        req = req.query(&[("q", query.trim())]);
    }

    let resp = req
        .send()
        .map_err(|e| format!("请求 Claude Marketplace 失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Claude Marketplace 返回 HTTP {}",
            resp.status().as_u16()
        ));
    }

    let mut data: ApiResponse = resp
        .json()
        .map_err(|e| format!("解析 Marketplace JSON 失败: {e}"))?;

    if let Some(src) = source_filter.filter(|s| !s.trim().is_empty()) {
        let src = src.trim();
        data.skills.retain(|s| s.source == src);
    }

    Ok(MarketplaceListResult {
        skills: data.skills,
        total: data.total,
        offset,
        limit,
    })
}
