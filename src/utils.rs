use super::ContentItem;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_value::Value;

/// Sorts given `items` by given `by` in given `order`. Supports top-level struct
/// keys as `by` as well as meta-level keys like `meta.date`.
pub fn sort_content_items(items: &mut Vec<ContentItem>, by: String, order: String) {
    items.sort_by(|a, b| {
        if by.contains("meta.") {
            let meta_key = by.replace("meta.", "");
            let comp_a = a.meta.get(&meta_key);
            let comp_b = b.meta.get(&meta_key);

            return if order == "desc" {
                comp_b.cmp(&comp_a)
            } else {
                comp_a.cmp(&comp_b)
            };
        } else {
            let comp_a: String = get_field_by_name(a, &by);
            let comp_b: String = get_field_by_name(b, &by);

            return if order == "desc" {
                comp_b.cmp(&comp_a)
            } else {
                comp_a.cmp(&comp_b)
            };
        }
    });
}

/// Returns a value of a given `s` by a given `field`. Enables the retrieval
/// of Struct values by key using a string.
pub fn get_field_by_name<T, R>(s: T, field: &str) -> R
    where
        T: Serialize,
        R: DeserializeOwned,
{
    let mut map = match serde_value::to_value(s) {
        Ok(Value::Map(map)) => map,
        _ => panic!("Not a struct."),
    };

    let key = Value::String(field.to_owned());
    let value = match map.remove(&key) {
        Some(value) => value,
        None => Value::String("".to_string()),
    };

    match R::deserialize(value) {
        Ok(r) => r,
        Err(_) => panic!("Something went wrong ..."),
    }
}

#[test]
fn test_get_field_by_name() {
    let item = ContentItem {
        path: "test-path".to_string(),
        slug: "test-slug".to_string(),
        meta: Default::default(),
        entry: "test-entry".to_string(),
        time_to_read: 0,
    };

    let path: String = get_field_by_name(item.clone(), "path");
    assert_eq!("test-path".to_string(), path);

    let slug: String = get_field_by_name(item.clone(), "slug");
    assert_eq!("test-slug".to_string(), slug);

    let entry: String = get_field_by_name(item.clone(), "entry");
    assert_eq!("test-entry".to_string(), entry);

    let time_to_read: u64 = get_field_by_name(item.clone(), "time_to_read");
    assert_eq!(0, time_to_read);

    let field_that_doesnt_exist: String = get_field_by_name(item.clone(), "doesnt_exist");
    assert_eq!("".to_string(), field_that_doesnt_exist);
}