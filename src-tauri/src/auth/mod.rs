//! 账号认证模块：微软正版登录（MSA -> Xbox Live -> XSTS -> Minecraft）。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

// MSA 公开客户端：官方启动器用的 app id，公开客户端不需要 client secret / PKCE。
const MS_CLIENT_ID: &str = "00000000402b5328"; // TODO(验证): 若微软收紧该 client_id，需换成自行申请的公开客户端
const MS_AUTHORIZE_URL: &str = "https://login.live.com/oauth20_authorize.srf";
const MS_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
const MS_REDIRECT_URI: &str = "https://login.live.com/oauth20_desktop.srf";
const MS_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";

const XBL_USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub uuid: String,
    pub name: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub uuid: String,
    pub name: String,
    pub expires_at: Option<i64>,
    pub has_refresh_token: bool,
    /// 是否是当前正在使用（启动游戏时取它的令牌）的账号。
    pub active: bool,
}

impl AccountView {
    pub fn of(account: &Account, active: bool) -> Self {
        Self {
            uuid: account.uuid.clone(),
            name: account.name.clone(),
            expires_at: account.expires_at,
            has_refresh_token: account.refresh_token.is_some(),
            active,
        }
    }
}

/// 本地账号库：一台设备可以存多个微软账号，同一时刻只有一个 active。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    #[serde(default)]
    pub active_uuid: Option<String>,
    #[serde(default)]
    pub accounts: Vec<Account>,
}

impl AccountStore {
    pub fn active(&self) -> Option<&Account> {
        let uuid = self.active_uuid.as_deref()?;
        self.accounts.iter().find(|a| a.uuid.eq_ignore_ascii_case(uuid))
    }

    pub fn views(&self) -> Vec<AccountView> {
        self.accounts
            .iter()
            .map(|account| {
                let active = self
                    .active_uuid
                    .as_deref()
                    .map(|uuid| uuid.eq_ignore_ascii_case(&account.uuid))
                    .unwrap_or(false);
                AccountView::of(account, active)
            })
            .collect()
    }

    /// 同一个 uuid 视为同一账号（重新登录即覆盖），按 uuid 去重。
    pub fn upsert(&mut self, account: &Account, make_active: bool) {
        match self
            .accounts
            .iter_mut()
            .find(|a| a.uuid.eq_ignore_ascii_case(&account.uuid))
        {
            Some(slot) => *slot = account.clone(),
            None => self.accounts.push(account.clone()),
        }
        if make_active || self.active_uuid.is_none() {
            self.active_uuid = Some(account.uuid.clone());
        }
    }

    /// active 指向的账号被删掉后，落到剩下的第一个账号上。
    fn repair_active(&mut self) {
        let valid = self.active_uuid.as_deref().is_some_and(|uuid| {
            self.accounts
                .iter()
                .any(|a| a.uuid.eq_ignore_ascii_case(uuid))
        });
        if !valid {
            self.active_uuid = self.accounts.first().map(|a| a.uuid.clone());
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct MsaToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub expires_in: u64,
}

#[derive(Deserialize, Debug)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Deserialize, Debug)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize, Debug)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize, Debug)]
struct MinecraftToken {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Deserialize, Debug)]
struct MinecraftProfile {
    id: String,
    name: String,
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

// RFC 3986 百分号编码，避免为两个参数引入 url crate。
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn authorize_url() -> String {
    format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}",
        MS_AUTHORIZE_URL,
        encode_component(MS_CLIENT_ID),
        encode_component(MS_SCOPE),
        encode_component(MS_REDIRECT_URI),
    )
}

fn xsts_error_hint(x_err: u64) -> String {
    match x_err {
        2148916227 => "该微软账号已被 Xbox Live 封禁".to_string(),
        2148916233 => "该微软账号还没有 Xbox 档案，请先到 xbox.com 创建角色".to_string(),
        2148916235 => "Xbox Live 在账号所在地区不可用".to_string(),
        2148916236 | 2148916237 => "该账号需要先完成年龄验证".to_string(),
        2148916238 => "未成年账号需加入家庭组并由成人批准".to_string(),
        other => format!("XSTS 拒绝授权（XErr {other}）"),
    }
}

pub async fn exchange_code_for_token(code: &str) -> Result<MsaToken, AppError> {
    let form = [
        ("client_id", MS_CLIENT_ID),
        ("code", code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", MS_REDIRECT_URI),
        ("scope", MS_SCOPE),
    ];

    let resp = http_client()
        .post(MS_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("换取微软令牌失败（网络层）: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AppError::Http(format!("读取微软令牌响应失败: {e}")))?;

    if !status.is_success() {
        return Err(AppError::Http(format!("换取微软令牌失败（HTTP {status}）: {body}")));
    }

    serde_json::from_str::<MsaToken>(&body)
        .map_err(|e| AppError::Http(format!("解析微软令牌响应失败: {e}；原始响应: {body}")))
}

pub async fn refresh_msa_token(refresh_token: &str) -> Result<MsaToken, AppError> {
    let form = [
        ("client_id", MS_CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
        ("redirect_uri", MS_REDIRECT_URI),
        ("scope", MS_SCOPE),
    ];

    let resp = http_client()
        .post(MS_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("刷新微软令牌失败（网络层）: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AppError::Http(format!("读取刷新响应失败: {e}")))?;

    if !status.is_success() {
        return Err(AppError::Http(format!(
            "刷新微软令牌失败（HTTP {status}），可能已过期，需要重新登录: {body}"
        )));
    }

    serde_json::from_str::<MsaToken>(&body)
        .map_err(|e| AppError::Http(format!("解析刷新响应失败: {e}；原始响应: {body}")))
}

async fn post_json(url: &str, body: &serde_json::Value, label: &str) -> Result<String, AppError> {
    let resp = http_client()
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("{label}失败（网络层）: {e}")))?;
    let status = resp.status();
    let raw = resp
        .text()
        .await
        .map_err(|e| AppError::Http(format!("读取{label}响应失败: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Http(format!("{label}失败（HTTP {status}）: {raw}")));
    }
    Ok(raw)
}

fn take_uhs(raw: &str, label: &str) -> Result<XboxResponse, AppError> {
    serde_json::from_str::<XboxResponse>(raw)
        .map_err(|e| AppError::Http(format!("解析{label}响应失败: {e}；原始响应: {raw}")))
}

pub async fn xbox_live_authenticate(msa_access_token: &str) -> Result<(String, String), AppError> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={msa_access_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });

    let raw = post_json(XBL_USER_AUTH_URL, &body, "Xbox Live 用户认证").await?;
    let parsed = take_uhs(&raw, "Xbox Live 用户认证")?;
    let uhs = parsed
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| AppError::Http("Xbox Live 响应里没有 uhs".to_string()))?;
    Ok((parsed.token, uhs))
}

pub async fn xsts_authorize(xbl_token: &str) -> Result<(String, String), AppError> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token],
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });

    let resp = http_client()
        .post(XSTS_AUTH_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("XSTS 授权失败（网络层）: {e}")))?;
    let status = resp.status();
    let raw = resp
        .text()
        .await
        .map_err(|e| AppError::Http(format!("读取 XSTS 响应失败: {e}")))?;

    if !status.is_success() {
        let x_err = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("XErr").and_then(|e| e.as_u64()));
        let message = match x_err {
            Some(code) => xsts_error_hint(code),
            None => format!("XSTS 授权失败（HTTP {status}）: {raw}"),
        };
        return Err(AppError::Http(message));
    }

    let parsed = take_uhs(&raw, "XSTS 授权")?;
    let uhs = parsed
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| AppError::Http("XSTS 响应里没有 uhs".to_string()))?;
    Ok((parsed.token, uhs))
}

pub async fn login_with_xbox(uhs: &str, xsts_token: &str) -> Result<(String, u64), AppError> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={uhs};{xsts_token}"),
    });

    let raw = post_json(MC_LOGIN_URL, &body, "登录 Minecraft 服务").await?;
    let parsed = serde_json::from_str::<MinecraftToken>(&raw)
        .map_err(|e| AppError::Http(format!("解析 Minecraft 令牌失败: {e}；原始响应: {raw}")))?;
    Ok((parsed.access_token, parsed.expires_in))
}

pub async fn fetch_profile(mc_access_token: &str) -> Result<(String, String), AppError> {
    let resp = http_client()
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_access_token)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("查询 Minecraft 档案失败（网络层）: {e}")))?;
    let status = resp.status();
    let raw = resp
        .text()
        .await
        .map_err(|e| AppError::Http(format!("读取 Minecraft 档案响应失败: {e}")))?;

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Http(
            "该账号没有 Minecraft: Java 版档案（未购买或档案未创建）".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(AppError::Http(format!("查询 Minecraft 档案失败（HTTP {status}）: {raw}")));
    }

    let parsed = serde_json::from_str::<MinecraftProfile>(&raw)
        .map_err(|e| AppError::Http(format!("解析 Minecraft 档案失败: {e}；原始响应: {raw}")))?;
    Ok((parsed.id, parsed.name))
}

async fn account_from_msa(msa: MsaToken) -> Result<Account, AppError> {
    let (xbl_token, _) = xbox_live_authenticate(&msa.access_token).await?;
    let (xsts_token, uhs) = xsts_authorize(&xbl_token).await?;
    let (mc_token, expires_in) = login_with_xbox(&uhs, &xsts_token).await?;
    let (uuid, name) = fetch_profile(&mc_token).await?;

    Ok(Account {
        uuid,
        name,
        access_token: Some(mc_token),
        refresh_token: msa.refresh_token,
        expires_at: Some(now_secs() + expires_in as i64),
    })
}

pub async fn refresh_account(account: &Account) -> Result<Account, AppError> {
    let refresh_token = account
        .refresh_token
        .as_deref()
        .ok_or_else(|| AppError::Config("本地账号没有 refresh_token，需要重新登录".to_string()))?;
    let msa = refresh_msa_token(refresh_token).await?;
    account_from_msa(msa).await
}

pub async fn get_valid_token(account: &Account) -> Result<String, AppError> {
    if let (Some(token), Some(expires_at)) = (account.access_token.as_deref(), account.expires_at) {
        if now_secs() + 120 < expires_at {
            return Ok(token.to_string());
        }
    }
    let refreshed = refresh_account(account).await?;
    update_account_silently(&refreshed)?;
    refreshed
        .access_token
        .ok_or_else(|| AppError::Internal("刷新后仍未拿到 Minecraft access_token".to_string()))
}

fn home_dir() -> Result<PathBuf, AppError> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 config 目录".to_string()))?;
    Ok(dir.join("NightSpark"))
}

fn store_path() -> Result<PathBuf, AppError> {
    Ok(home_dir()?.join("accounts.json"))
}

/// 0.1.0 时代的单账号文件，只在首次读取时做一次性迁移。
fn legacy_account_path() -> Result<PathBuf, AppError> {
    Ok(home_dir()?.join("account.json"))
}

pub fn load_store() -> Result<AccountStore, AppError> {
    let path = store_path()?;
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        let mut store = serde_json::from_str::<AccountStore>(&raw).map_err(|e| {
            AppError::Config(format!("解析本地账号失败（{}）: {e}", path.display()))
        })?;
        store.repair_active();
        return Ok(store);
    }

    // 旧版单账号文件迁移：迁移成功后把原文件改名为 .bak，避免下次重复迁移。
    let legacy = legacy_account_path()?;
    if legacy.exists() {
        let raw = std::fs::read_to_string(&legacy)?;
        if let Ok(account) = serde_json::from_str::<Account>(&raw) {
            let mut store = AccountStore::default();
            store.upsert(&account, true);
            save_store(&store)?;
            let _ = std::fs::rename(&legacy, legacy.with_extension("json.bak"));
            return Ok(store);
        }
    }

    Ok(AccountStore::default())
}

pub fn save_store(store: &AccountStore) -> Result<(), AppError> {
    let path = store_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| AppError::Config(format!("序列化账号失败: {e}")))?;
    // 先写临时文件再替换，避免写一半掉电把账号库写坏。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 当前使用的账号（启动游戏、界面展示都取它）。
pub fn load_account() -> Result<Option<Account>, AppError> {
    Ok(load_store()?.active().cloned())
}

pub fn list_accounts() -> Result<Vec<AccountView>, AppError> {
    Ok(load_store()?.views())
}

/// 登录成功后落库：按 uuid 覆盖或新增，并把新账号设为当前账号。
pub fn save_account(account: &Account) -> Result<(), AppError> {
    let mut store = load_store()?;
    store.upsert(account, true);
    save_store(&store)
}

/// 后台静默刷新落库：不改变当前账号，避免刷新把用户手选的账号顶掉。
pub fn update_account_silently(account: &Account) -> Result<(), AppError> {
    let mut store = load_store()?;
    store.upsert(account, false);
    save_store(&store)
}

pub fn switch_account(uuid: &str) -> Result<AccountView, AppError> {
    let mut store = load_store()?;
    let target = store
        .accounts
        .iter()
        .find(|a| a.uuid.eq_ignore_ascii_case(uuid))
        .cloned()
        .ok_or_else(|| AppError::NotFound("本地账号库里没有这个账号".to_string()))?;
    store.active_uuid = Some(target.uuid.clone());
    save_store(&store)?;
    Ok(AccountView::of(&target, true))
}

pub fn remove_account(uuid: &str) -> Result<(), AppError> {
    let mut store = load_store()?;
    store
        .accounts
        .retain(|a| !a.uuid.eq_ignore_ascii_case(uuid));
    store.repair_active();
    save_store(&store)
}

/// 退出登录：移除当前账号，其它账号保留。
pub fn clear_account() -> Result<(), AppError> {
    match load_store()?.active_uuid {
        Some(uuid) => remove_account(&uuid),
        None => Ok(()),
    }
}

/// 令牌是否需要在过期前提前刷新（留 5 分钟余量，避免启动瞬间过期）。
pub fn needs_refresh(account: &Account) -> bool {
    match account.expires_at {
        Some(expires_at) => now_secs() + 300 >= expires_at,
        None => account.access_token.is_none(),
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    fn authorize_url_has_required_params() {
        let url = authorize_url();
        println!("authorize url: {url}");
        assert!(url.contains("client_id=00000000402b5328"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
    }

    #[tokio::test]
    #[ignore = "真机联网：打微软 token 端点验证 OAuth 参数"]
    async fn probe_token_endpoint_accepts_oauth_params() {
        let outcome = exchange_code_for_token("fake-code-for-probe").await;
        let message = match outcome {
            Ok(token) => format!("假 code 竟然换到了令牌: {}", token.access_token),
            Err(err) => format!("{err}"),
        };
        println!("token 端点响应: {message}");
        assert!(
            message.contains("invalid_grant"),
            "微软没有返回 invalid_grant，说明 client_id/redirect/scope/grant_type 有问题: {message}"
        );
    }

    #[tokio::test]
    #[ignore = "真机联网：验证刷新流程同样被端点接受参数"]
    async fn probe_refresh_endpoint_accepts_oauth_params() {
        let outcome = refresh_msa_token("fake-refresh-token").await;
        let message = match outcome {
            Ok(token) => format!("假 refresh_token 竟然换到了令牌: {}", token.access_token),
            Err(err) => format!("{err}"),
        };
        println!("刷新端点响应: {message}");
        assert!(
            message.contains("invalid_grant") || message.contains("invalid_request"),
            "刷新参数可能不合法: {message}"
        );
    }
}
