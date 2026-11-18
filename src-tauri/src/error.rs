//! 统一错误类型。
//!
//! 所有 Tauri command 统一返回 `Result<T, AppError>`，
//! AppError 序列化为 `{ "code": "...", "message": "..." }` 交给前端。

use std::error::Error;
use std::fmt;

/// 应用统一错误。
#[derive(Debug)]
pub enum AppError {
    /// 底层 I/O 错误
    Io(std::io::Error),
    /// 网络 / HTTP 相关错误
    Http(String),
    /// 配置读写 / 解析错误
    Config(String),
    /// 目标资源不存在
    NotFound(String),
    /// 内部逻辑错误
    Internal(String),
}

impl AppError {
    /// 供序列化使用的固定短错误码。
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
            AppError::Http(_) => "http",
            AppError::Config(_) => "config",
            AppError::NotFound(_) => "not_found",
            AppError::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Io(err) => write!(f, "io error: {}", err),
            AppError::Http(msg) => write!(f, "http error: {}", msg),
            AppError::Config(msg) => write!(f, "config error: {}", msg),
            AppError::NotFound(msg) => write!(f, "not found: {}", msg),
            AppError::Internal(msg) => write!(f, "internal error: {}", msg),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            AppError::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// 手写 Serialize：序列化为 `{ code, message }`。
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

