use serde_json::Value;

use crate::http_support::ApiError;

// Run only when decoding legacy codecs. Their canonical shape must not silently
// acquire semantics introduced by Game 1, even when a current DTO can read them.
pub(super) fn reject_game_one_fields(serialized: &str) -> Result<(), ApiError> {
    let value: Value = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("legacy codec shape", error))?;
    if has_game_one_fields(&value) {
        Err(ApiError::internal())
    } else {
        Ok(())
    }
}

fn has_game_one_fields(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(has_game_one_fields),
        Value::Object(fields) => {
            fields.contains_key("preparation_samples")
                || fields.contains_key("drawing_blocked")
                || fields.get("kind").and_then(Value::as_str) == Some("dark_arts")
                || matches!(
                    fields.get("type").and_then(Value::as_str),
                    Some(
                        "reaction_effect"
                            | "random_sampled"
                            | "drawing_blocked"
                            | "drawing_restored"
                    )
                )
                || (fields.get("type").and_then(Value::as_str) == Some("pile_shuffled")
                    && fields.contains_key("samples"))
                || (fields.get("type").and_then(Value::as_str) == Some("no_op")
                    && fields.get("reason").and_then(Value::as_str) == Some("drawing_blocked"))
                || (fields.get("type").and_then(Value::as_str) == Some("card_acquired")
                    && fields
                        .get("effects")
                        .and_then(Value::as_array)
                        .is_some_and(|effects| {
                            effects.iter().any(|effect| {
                                effect["type"] == "moved"
                                    && effect["from"] == "market"
                                    && effect["to"] == "hero_draw_pile"
                            })
                        }))
                || fields.values().any(has_game_one_fields)
        }
        _ => false,
    }
}
