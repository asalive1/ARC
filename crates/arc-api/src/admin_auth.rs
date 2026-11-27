use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::Extension,
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use arc_infrastructure::postgres::PgPool;

const SESSION_COOKIE: &str = "arc_admin_session";
const SESSION_LIFETIME_SECONDS: i64 = 12 * 60 * 60;
const MAX_LOGIN_FAILURES: i32 = 5;
const LOGIN_FAILURE_WINDOW_MINUTES: i64 = 15;
const LOGIN_BLOCK_MINUTES: i64 = 15;

#[derive(Clone)]
pub struct AdminAuth {
    pool: PgPool,
    secure_cookie: bool,
}

#[derive(Clone)]
pub struct AdminSession {
    pub user_id: Uuid,
    pub username: String,
    csrf_token: String,
}

#[derive(Debug)]
pub(crate) struct AuthApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

#[derive(Deserialize)]
struct BootstrapRequest {
    bootstrap_token: String,
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct RecoveryRequest {
    recovery_token: String,
    new_password: String,
}

#[derive(Deserialize)]
pub(crate) struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Serialize)]
struct SessionResponse {
    username: String,
    csrf_token: String,
}

impl AdminAuth {
    #[cfg(test)]
    pub(crate) fn for_test(pool: PgPool) -> Self {
        Self { pool, secure_cookie: false }
    }

    pub async fn from_env(pool: PgPool) -> Result<Self, Box<dyn std::error::Error>> {
        let secure_cookie = match std::env::var("ARC_ADMIN_COOKIE_SECURE") {
            Ok(value) if value.eq_ignore_ascii_case("true") => true,
            Ok(value) if value.eq_ignore_ascii_case("false") => false,
            Ok(_) => return Err("ARC_ADMIN_COOKIE_SECURE must be true or false".into()),
            Err(std::env::VarError::NotPresent) => true,
            Err(error) => return Err(error.into()),
        };

        let state_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM arc_admin_auth_state WHERE singleton = TRUE)",
        )
        .fetch_one(&pool)
        .await?;
        if !state_exists {
            let bootstrap_token = read_optional_secret_file("ARC_ADMIN_BOOTSTRAP_TOKEN_FILE")?
                .ok_or("ARC_ADMIN_BOOTSTRAP_TOKEN_FILE is required for first-run provisioning")?;
            let recovery_token = read_optional_secret_file("ARC_ADMIN_RECOVERY_TOKEN_FILE")?
                .ok_or("ARC_ADMIN_RECOVERY_TOKEN_FILE is required for first-run recovery")?;
            validate_secret_token(&bootstrap_token).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "administrator bootstrap token has an invalid format",
                )
            })?;
            validate_secret_token(&recovery_token).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "administrator recovery token has an invalid format",
                )
            })?;
            sqlx::query(
                "INSERT INTO arc_admin_auth_state \
                 (singleton, bootstrap_token_hash, recovery_token_hash) \
                 VALUES (TRUE, $1, $2)",
            )
            .bind(token_hash(&bootstrap_token))
            .bind(token_hash(&recovery_token))
            .execute(&pool)
            .await?;
        }

        let admin_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM arc_admin_users)")
                .fetch_one(&pool)
                .await?;
        if !admin_exists {
            let bootstrap_available: bool = sqlx::query_scalar(
                "SELECT bootstrap_consumed_at IS NULL \
                 FROM arc_admin_auth_state WHERE singleton = TRUE",
            )
            .fetch_one(&pool)
            .await?;
            if !bootstrap_available {
                return Err(
                    "administrator provisioning is required but the bootstrap token was consumed"
                        .into(),
                );
            }
        }

        Ok(Self {
            pool,
            secure_cookie,
        })
    }

    fn cookie(&self, token: &str) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!(
            "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_LIFETIME_SECONDS}{secure}"
        )
    }

    fn expired_cookie(&self) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
    }
}

pub(crate) fn public_routes(auth: AdminAuth) -> Router {
    Router::new()
        .route("/api/v1/auth/status", axum::routing::get(status))
        .route("/api/v1/auth/session", axum::routing::get(current_session))
        .route("/api/v1/auth/bootstrap", post(bootstrap))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/recover", post(recover))
        .layer(Extension(auth))
}

pub(crate) fn protect(router: Router, auth: AdminAuth) -> Router {
    router.route_layer(middleware::from_fn_with_state(auth, require_admin))
}

pub(crate) async fn change_password(
    Extension(auth): Extension<AdminAuth>,
    Extension(session): Extension<AdminSession>,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Response, AuthApiError> {
    validate_password(&request.new_password)?;
    let row = sqlx::query("SELECT password_hash FROM arc_admin_users WHERE id = $1")
        .bind(session.user_id)
        .fetch_optional(&auth.pool)
        .await?
        .ok_or_else(AuthApiError::unauthorized)?;
    let stored_hash: String = row
        .try_get("password_hash")
        .map_err(AuthApiError::database)?;
    if !verify_password(request.current_password, stored_hash).await? {
        return Err(AuthApiError::unauthorized());
    }
    let password_hash = hash_password(request.new_password).await?;
    let mut transaction = auth.pool.begin().await?;
    sqlx::query(
        "UPDATE arc_admin_users SET password_hash = $1, updated_at = clock_timestamp() \
         WHERE id = $2",
    )
    .bind(password_hash)
    .bind(session.user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE arc_admin_sessions SET revoked_at = clock_timestamp() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(session.user_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let mut response = Json(json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth.expired_cookie()).map_err(|_| AuthApiError::unavailable())?,
    );
    Ok(response)
}

async fn status(
    Extension(auth): Extension<AdminAuth>,
) -> Result<Json<serde_json::Value>, AuthApiError> {
    let user_exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM arc_admin_users)")
        .fetch_one(&auth.pool)
        .await?;
    let bootstrap_required: bool = if user_exists {
        false
    } else {
        sqlx::query_scalar(
            "SELECT bootstrap_consumed_at IS NULL \
             FROM arc_admin_auth_state WHERE singleton = TRUE",
        )
        .fetch_one(&auth.pool)
        .await?
    };
    Ok(Json(json!({
        "bootstrap_required": bootstrap_required,
        "login_required": user_exists,
    })))
}

async fn current_session(
    Extension(auth): Extension<AdminAuth>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthApiError> {
    let session = load_session(&auth, &headers)
        .await?
        .ok_or_else(AuthApiError::unauthorized)?;
    Ok(Json(SessionResponse {
        username: session.username,
        csrf_token: session.csrf_token,
    }))
}

async fn bootstrap(
    Extension(auth): Extension<AdminAuth>,
    Json(request): Json<BootstrapRequest>,
) -> Result<Response, AuthApiError> {
    validate_username(&request.username)?;
    validate_password(&request.password)?;
    validate_secret_token(&request.bootstrap_token).map_err(|_| AuthApiError::unauthorized())?;
    let preliminary_state = sqlx::query(
        "SELECT bootstrap_token_hash, bootstrap_consumed_at IS NULL AS available \
         FROM arc_admin_auth_state WHERE singleton = TRUE",
    )
    .fetch_optional(&auth.pool)
    .await?
    .ok_or_else(AuthApiError::unavailable)?;
    let preliminary_hash: Vec<u8> = preliminary_state
        .try_get("bootstrap_token_hash")
        .map_err(AuthApiError::database)?;
    let available: bool = preliminary_state
        .try_get("available")
        .map_err(AuthApiError::database)?;
    if !available || !constant_time_eq(&preliminary_hash, &token_hash(&request.bootstrap_token)) {
        return Err(AuthApiError::unauthorized());
    }
    let password_hash = hash_password(request.password).await?;
    let mut transaction = auth.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(735104201)")
        .execute(&mut *transaction)
        .await?;
    let state = sqlx::query(
        "SELECT bootstrap_token_hash, bootstrap_consumed_at \
         FROM arc_admin_auth_state WHERE singleton = TRUE FOR UPDATE",
    )
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(AuthApiError::unavailable)?;
    let consumed_at: Option<chrono::DateTime<chrono::Utc>> = state
        .try_get("bootstrap_consumed_at")
        .map_err(AuthApiError::database)?;
    let stored_hash: Vec<u8> = state
        .try_get("bootstrap_token_hash")
        .map_err(AuthApiError::database)?;
    let submitted_hash = token_hash(&request.bootstrap_token);
    if consumed_at.is_some() || !constant_time_eq(&stored_hash, &submitted_hash) {
        return Err(AuthApiError::unauthorized());
    }
    let user_exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM arc_admin_users)")
        .fetch_one(&mut *transaction)
        .await?;
    if user_exists {
        return Err(AuthApiError::conflict());
    }
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO arc_admin_users (id, username, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(request.username.trim().to_ascii_lowercase())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE arc_admin_auth_state SET bootstrap_consumed_at = clock_timestamp() \
         WHERE singleton = TRUE",
    )
    .execute(&mut *transaction)
    .await?;
    let session_token = random_token();
    let csrf_token = random_token();
    insert_session(&mut transaction, user_id, &session_token, &csrf_token).await?;
    transaction.commit().await?;
    session_response(&auth, session_token, csrf_token)
}

async fn login(
    Extension(auth): Extension<AdminAuth>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, AuthApiError> {
    validate_username(&request.username)?;
    if request.password.is_empty() || request.password.len() > 1024 {
        return Err(AuthApiError::unauthorized());
    }
    let username = request.username.trim().to_ascii_lowercase();
    if login_blocked(&auth.pool, &username).await? {
        return Err(AuthApiError::unauthorized());
    }
    let row =
        sqlx::query("SELECT id, username, password_hash FROM arc_admin_users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&auth.pool)
            .await?;
    let user = row
        .map(|row| {
            Ok::<_, AuthApiError>((
                row.try_get::<Uuid, _>("id")
                    .map_err(AuthApiError::database)?,
                row.try_get::<String, _>("username")
                    .map_err(AuthApiError::database)?,
                row.try_get::<String, _>("password_hash")
                    .map_err(AuthApiError::database)?,
            ))
        })
        .transpose()?;
    let stored_hash = user
        .as_ref()
        .map(|(_, _, hash)| hash.clone())
        .unwrap_or_else(dummy_password_hash);
    let valid = verify_password(request.password, stored_hash).await?;
    let Some((user_id, username, _)) = user.filter(|_| valid) else {
        record_login_failure(&auth.pool, &username).await?;
        return Err(AuthApiError::unauthorized());
    };
    sqlx::query("DELETE FROM arc_admin_login_throttles WHERE username = $1")
        .bind(&username)
        .execute(&auth.pool)
        .await?;
    let session_token = random_token();
    let csrf_token = random_token();
    let mut transaction = auth.pool.begin().await?;
    insert_session(&mut transaction, user_id, &session_token, &csrf_token).await?;
    transaction.commit().await?;
    session_response(&auth, session_token, csrf_token)
}

async fn logout(
    Extension(auth): Extension<AdminAuth>,
    headers: HeaderMap,
) -> Result<Response, AuthApiError> {
    let session = load_session(&auth, &headers)
        .await?
        .ok_or_else(AuthApiError::unauthorized)?;
    validate_csrf(&headers, &session)?;
    let token = session_cookie_token(&headers).ok_or_else(AuthApiError::unauthorized)?;
    sqlx::query(
        "UPDATE arc_admin_sessions SET revoked_at = clock_timestamp() \
         WHERE session_token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash(token))
    .execute(&auth.pool)
    .await?;
    let mut response = Json(json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth.expired_cookie()).map_err(|_| AuthApiError::unavailable())?,
    );
    Ok(response)
}

async fn recover(
    Extension(auth): Extension<AdminAuth>,
    Json(request): Json<RecoveryRequest>,
) -> Result<Response, AuthApiError> {
    validate_password(&request.new_password)?;
    validate_secret_token(&request.recovery_token).map_err(|_| AuthApiError::unauthorized())?;
    let preliminary_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT recovery_token_hash FROM arc_admin_auth_state WHERE singleton = TRUE",
    )
    .fetch_optional(&auth.pool)
    .await?
    .ok_or_else(AuthApiError::unavailable)?;
    if !constant_time_eq(&preliminary_hash, &token_hash(&request.recovery_token)) {
        return Err(AuthApiError::unauthorized());
    }
    let password_hash = hash_password(request.new_password).await?;
    let mut transaction = auth.pool.begin().await?;
    let state = sqlx::query(
        "SELECT recovery_token_hash FROM arc_admin_auth_state \
         WHERE singleton = TRUE FOR UPDATE",
    )
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(AuthApiError::unavailable)?;
    let stored_hash: Vec<u8> = state
        .try_get("recovery_token_hash")
        .map_err(AuthApiError::database)?;
    let submitted_hash = token_hash(&request.recovery_token);
    if !constant_time_eq(&stored_hash, &submitted_hash) {
        return Err(AuthApiError::unauthorized());
    }
    let user = sqlx::query(
        "SELECT id, username FROM arc_admin_users ORDER BY created_at LIMIT 1 FOR UPDATE",
    )
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(AuthApiError::unavailable)?;
    let user_id: Uuid = user.try_get("id").map_err(AuthApiError::database)?;
    let username: String = user.try_get("username").map_err(AuthApiError::database)?;
    let recovery_token = random_token();
    sqlx::query("UPDATE arc_admin_auth_state SET recovery_token_hash = $1 WHERE singleton = TRUE")
        .bind(token_hash(&recovery_token))
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE arc_admin_users SET password_hash = $1, updated_at = clock_timestamp() \
         WHERE id = $2",
    )
    .bind(password_hash)
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE arc_admin_sessions SET revoked_at = clock_timestamp() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    let session_token = random_token();
    let csrf_token = random_token();
    insert_session(&mut transaction, user_id, &session_token, &csrf_token).await?;
    transaction.commit().await?;
    let mut response = Json(json!({
        "username": username,
        "csrf_token": csrf_token,
        "replacement_recovery_token": recovery_token,
        "recovery_token_warning": "Save this token securely; it is shown only once."
    }))
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth.cookie(&session_token))
            .map_err(|_| AuthApiError::unavailable())?,
    );
    Ok(response)
}

async fn require_admin(
    axum::extract::State(auth): axum::extract::State<AdminAuth>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path == "/api/v1/health/live" || path == "/api/v1/health/ready" {
        return next.run(request).await;
    }
    let session = match load_session(&auth, request.headers()).await {
        Ok(Some(session)) => session,
        Ok(None) => return AuthApiError::unauthorized().into_response(),
        Err(error) => return error.into_response(),
    };
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        if let Err(error) = validate_csrf(request.headers(), &session) {
            return error.into_response();
        }
    }
    request.extensions_mut().insert(session);
    next.run(request).await
}

pub(crate) async fn load_session(
    auth: &AdminAuth,
    headers: &HeaderMap,
) -> Result<Option<AdminSession>, AuthApiError> {
    let Some(token) = session_cookie_token(headers) else {
        return Ok(None);
    };
    let row = sqlx::query(
        "SELECT u.id, u.username, s.csrf_token \
         FROM arc_admin_sessions s \
         JOIN arc_admin_users u ON u.id = s.user_id \
         WHERE s.session_token_hash = $1 AND s.revoked_at IS NULL \
           AND s.expires_at > clock_timestamp()",
    )
    .bind(token_hash(token))
    .fetch_optional(&auth.pool)
    .await?;
    row.map(|row| {
        Ok(AdminSession {
            user_id: row.try_get("id").map_err(AuthApiError::database)?,
            username: row.try_get("username").map_err(AuthApiError::database)?,
            csrf_token: row.try_get("csrf_token").map_err(AuthApiError::database)?,
        })
    })
    .transpose()
}

fn session_cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE && !value.is_empty()).then_some(value))
}

fn validate_csrf(headers: &HeaderMap, session: &AdminSession) -> Result<(), AuthApiError> {
    let provided = headers
        .get("x-arc-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !bool::from(provided.as_bytes().ct_eq(session.csrf_token.as_bytes())) {
        return Err(AuthApiError::forbidden());
    }
    Ok(())
}

fn session_response(
    auth: &AdminAuth,
    session_token: String,
    csrf_token: String,
) -> Result<Response, AuthApiError> {
    let mut response = Json(json!({ "csrf_token": csrf_token })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth.cookie(&session_token))
            .map_err(|_| AuthApiError::unavailable())?,
    );
    Ok(response)
}

async fn insert_session(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    session_token: &str,
    csrf_token: &str,
) -> Result<(), AuthApiError> {
    sqlx::query(
        "INSERT INTO arc_admin_sessions \
         (id, user_id, session_token_hash, csrf_token, expires_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp() + ($5 * INTERVAL '1 second'))",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(token_hash(session_token))
    .bind(csrf_token)
    .bind(SESSION_LIFETIME_SECONDS)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn login_blocked(pool: &PgPool, username: &str) -> Result<bool, AuthApiError> {
    sqlx::query_scalar(
        "SELECT COALESCE(blocked_until > clock_timestamp(), FALSE) \
         FROM arc_admin_login_throttles WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map(|blocked| blocked.unwrap_or(false))
    .map_err(AuthApiError::database)
}

async fn record_login_failure(pool: &PgPool, username: &str) -> Result<(), AuthApiError> {
    sqlx::query(
        "INSERT INTO arc_admin_login_throttles \
         (username, failures, window_started_at, blocked_until) \
         VALUES ($1, 1, clock_timestamp(), NULL) \
         ON CONFLICT (username) DO UPDATE SET \
           failures = CASE \
             WHEN arc_admin_login_throttles.window_started_at \
                  < clock_timestamp() - ($2 * INTERVAL '1 minute') THEN 1 \
             ELSE arc_admin_login_throttles.failures + 1 \
           END, \
           window_started_at = CASE \
             WHEN arc_admin_login_throttles.window_started_at \
                  < clock_timestamp() - ($2 * INTERVAL '1 minute') THEN clock_timestamp() \
             ELSE arc_admin_login_throttles.window_started_at \
           END, \
           blocked_until = CASE \
             WHEN arc_admin_login_throttles.window_started_at \
                  < clock_timestamp() - ($2 * INTERVAL '1 minute') THEN NULL \
             WHEN arc_admin_login_throttles.failures + 1 >= $3 \
               AND (arc_admin_login_throttles.blocked_until IS NULL \
                    OR arc_admin_login_throttles.blocked_until <= clock_timestamp()) \
               THEN clock_timestamp() + ($4 * INTERVAL '1 minute') \
             ELSE arc_admin_login_throttles.blocked_until \
           END",
    )
    .bind(username)
    .bind(LOGIN_FAILURE_WINDOW_MINUTES)
    .bind(MAX_LOGIN_FAILURES)
    .bind(LOGIN_BLOCK_MINUTES)
    .execute(pool)
    .await?;
    Ok(())
}

async fn hash_password(password: String) -> Result<String, AuthApiError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| AuthApiError::unavailable())
    })
    .await
    .map_err(|_| AuthApiError::unavailable())?
}

async fn verify_password(password: String, stored_hash: String) -> Result<bool, AuthApiError> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&stored_hash).map_err(|_| AuthApiError::unavailable())?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(|_| AuthApiError::unavailable())?
}

fn dummy_password_hash() -> String {
    "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        .to_owned()
}

fn validate_username(username: &str) -> Result<(), AuthApiError> {
    let normalized = username.trim();
    if !(3..=64).contains(&normalized.len())
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AuthApiError::bad_request());
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), AuthApiError> {
    if password.len() < 12 || password.len() > 128 || password.chars().any(char::is_control) {
        return Err(AuthApiError::bad_request());
    }
    Ok(())
}

fn validate_secret_token(token: &str) -> Result<(), &'static str> {
    if token.len() < 32
        || token.len() > 256
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("secret token has an invalid format");
    }
    Ok(())
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn read_optional_secret_file(name: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match std::env::var(name) {
        Ok(path) if path.trim().is_empty() => {
            Err(format!("{name} must not be empty when set").into())
        }
        Ok(path) => {
            let value = std::fs::read_to_string(path)?;
            let value = value.trim().to_owned();
            if value.is_empty() {
                return Err(format!("{name} secret file is empty").into());
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl AuthApiError {
    fn bad_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: "request values do not meet the required format",
        }
    }

    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "authentication_required",
            message: "administrator credentials were rejected",
        }
    }

    fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "csrf_validation_failed",
            message: "request origin validation failed",
        }
    }

    fn conflict() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "administrator_already_provisioned",
            message: "administrator provisioning has already completed",
        }
    }

    fn unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "authentication_unavailable",
            message: "administrator authentication is temporarily unavailable",
        }
    }

    fn database(error: sqlx::Error) -> Self {
        tracing::error!(%error, "administrator authentication database operation failed");
        Self::unavailable()
    }
}

impl From<sqlx::Error> for AuthApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::database(error)
    }
}

impl IntoResponse for AuthApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn administrator_input_validation_enforces_local_account_requirements() {
        assert!(validate_username("  ops-admin_1  ").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username("admin/name").is_err());
        assert!(validate_password("correct horse battery staple").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password("password with\nnewline").is_err());
    }

    #[test]
    fn bootstrap_tokens_require_high_entropy_url_safe_input() {
        assert!(validate_secret_token(&"a".repeat(64)).is_ok());
        assert!(validate_secret_token("too-short").is_err());
        assert!(validate_secret_token(&format!("{}=", "a".repeat(64))).is_err());
    }

    #[test]
    fn token_hash_comparison_accepts_only_identical_hashes() {
        let expected = token_hash("token-a");
        let matching = token_hash("token-a");
        let different = token_hash("token-b");
        assert!(constant_time_eq(&expected, &matching));
        assert!(!constant_time_eq(&expected, &different));
    }
}
