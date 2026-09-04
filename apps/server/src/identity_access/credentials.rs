use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

use crate::http_support::ApiError;

pub(super) async fn verify_password(
    password: String,
    stored_hash: String,
) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&stored_hash).map_err(|_| ())?;
        Ok::<_, ()>(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok(),
        )
    })
    .await
    .map_err(|error| ApiError::internal_with("identity access application operation", error))?
    .map_err(|()| ApiError::internal())
}

pub(super) async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|_| ())
    })
    .await
    .map_err(|error| ApiError::internal_with("identity access application operation", error))?
    .map_err(|()| ApiError::internal())
}

pub(super) fn validate_display_name(display_name: &str) -> Result<&str, ApiError> {
    let normalized = display_name.trim();
    if normalized.is_empty()
        || normalized.chars().count() > 40
        || normalized.chars().any(char::is_control)
    {
        return Err(ApiError::invalid_display_name());
    }

    Ok(normalized)
}

pub(super) fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() > 128 || weak_password(password) {
        return Err(ApiError::weak_password());
    }

    Ok(())
}

fn weak_password(password: &str) -> bool {
    const COMMON_FRAGMENTS: [&str; 7] = [
        "password",
        "qwerty",
        "123456",
        "abcdef",
        "senha",
        "harrypotter",
        "hogwarts",
    ];
    let normalized = password.to_lowercase();

    password.chars().count() < 12
        || distinct_character_count(password, 4) < 4
        || COMMON_FRAGMENTS
            .iter()
            .any(|candidate| normalized.contains(candidate))
        || is_repeated_pattern(password)
        || contains_ascii_sequence(&normalized, 4)
}

fn distinct_character_count(value: &str, limit: usize) -> usize {
    let mut distinct = Vec::with_capacity(limit);
    for character in value.chars() {
        if !distinct.contains(&character) {
            distinct.push(character);
            if distinct.len() == limit {
                break;
            }
        }
    }
    distinct.len()
}

fn is_repeated_pattern(value: &str) -> bool {
    let characters: Vec<char> = value.chars().collect();
    (1..=characters.len() / 2).any(|pattern_length| {
        characters.len().is_multiple_of(pattern_length)
            && characters
                .iter()
                .enumerate()
                .all(|(index, character)| *character == characters[index % pattern_length])
    })
}

fn contains_ascii_sequence(value: &str, minimum_length: usize) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(minimum_length).any(|window| {
        window
            .windows(2)
            .all(|pair| pair[1].is_ascii_alphanumeric() && pair[1] == pair[0].wrapping_add(1))
            || window
                .windows(2)
                .all(|pair| pair[1].is_ascii_alphanumeric() && pair[0] == pair[1].wrapping_add(1))
    })
}
