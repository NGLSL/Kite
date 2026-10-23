use std::fmt;
use std::io;

pub type ServiceResult<T> = Result<T, ServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    AccessDenied,
    NeedsInstall,
    NotInstalled,
    NotRunning,
    InvalidRequest,
    Authentication,
    Protocol,
    Pipe,
    Service,
    Backend,
    Cancelled,
    PathMismatch,
    Internal,
}

#[derive(Debug)]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub message: String,
    pub code: Option<u32>,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: None,
        }
    }
    pub fn with_code(kind: ErrorKind, message: impl Into<String>, code: u32) -> Self {
        Self {
            kind,
            message: message.into(),
            code: Some(code),
        }
    }
    pub fn needs_install(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NeedsInstall, message)
    }
    pub fn not_installed() -> Self {
        Self::new(ErrorKind::NotInstalled, "Kite 文件索引服务尚未安装")
    }
    pub fn access_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::AccessDenied, message)
    }
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidRequest, message)
    }
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Authentication, message)
    }
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, message)
    }
    pub fn pipe(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Pipe, message)
    }
    pub fn service(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Service, message)
    }
    pub fn backend(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Backend, message)
    }
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Cancelled, message)
    }
    pub fn remote(kind: String, message: String, code: Option<u32>) -> Self {
        let kind = match kind.as_str() {
            "access_denied" => ErrorKind::AccessDenied,
            "needs_install" => ErrorKind::NeedsInstall,
            "not_installed" => ErrorKind::NotInstalled,
            "not_running" => ErrorKind::NotRunning,
            "invalid_request" => ErrorKind::InvalidRequest,
            "authentication" => ErrorKind::Authentication,
            "protocol" => ErrorKind::Protocol,
            "pipe" => ErrorKind::Pipe,
            "service" => ErrorKind::Service,
            "backend" => ErrorKind::Backend,
            "cancelled" => ErrorKind::Cancelled,
            "path_mismatch" => ErrorKind::PathMismatch,
            "internal" => ErrorKind::Internal,
            _ => ErrorKind::Internal,
        };
        Self {
            kind,
            message,
            code,
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.code {
            write!(
                f,
                "{} ({}; Win32 {})",
                self.message,
                format_kind(self.kind),
                code
            )
        } else {
            write!(f, "{} ({})", self.message, format_kind(self.kind))
        }
    }
}

fn format_kind(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::AccessDenied => "access_denied",
        ErrorKind::NeedsInstall => "needs_install",
        ErrorKind::NotInstalled => "not_installed",
        ErrorKind::NotRunning => "not_running",
        ErrorKind::InvalidRequest => "invalid_request",
        ErrorKind::Authentication => "authentication",
        ErrorKind::Protocol => "protocol",
        ErrorKind::Pipe => "pipe",
        ErrorKind::Service => "service",
        ErrorKind::Backend => "backend",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::PathMismatch => "path_mismatch",
        ErrorKind::Internal => "internal",
    }
}

impl std::error::Error for ServiceError {}

impl From<io::Error> for ServiceError {
    fn from(error: io::Error) -> Self {
        let code = error.raw_os_error().map(|value| value as u32);
        let kind = match (error.kind(), code) {
            (io::ErrorKind::PermissionDenied, _) => ErrorKind::AccessDenied,
            (io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof, _)
            | (_, Some(109 | 232 | 233 | 535)) => ErrorKind::Pipe,
            (_, Some(1223 | 995)) => ErrorKind::Cancelled,
            _ => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.to_string(),
            code,
        }
    }
}

impl From<windows::core::Error> for ServiceError {
    fn from(error: windows::core::Error) -> Self {
        let hr = error.code().0 as u32;
        let code = hr & 0xffff;
        let kind = match code {
            5 | 1314 => ErrorKind::AccessDenied,
            1223 => ErrorKind::Cancelled,
            1060 | 1073 => ErrorKind::NotInstalled,
            1062 => ErrorKind::NotRunning,
            109 | 232 | 233 | 535 => ErrorKind::Pipe,
            _ => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.message().to_string(),
            code: Some(code),
        }
    }
}

pub(crate) fn win32_error(code: u32, context: impl Into<String>) -> ServiceError {
    let kind = match code {
        5 | 1314 => ErrorKind::AccessDenied,
        1223 => ErrorKind::Cancelled,
        1060 | 1073 => ErrorKind::NotInstalled,
        1062 => ErrorKind::NotRunning,
        109 | 232 | 233 | 535 => ErrorKind::Pipe,
        _ => ErrorKind::Internal,
    };
    ServiceError::with_code(kind, context, code)
}
