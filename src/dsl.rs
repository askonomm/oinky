use super::ContentItem;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentDSLItem {
    pub name: String,
    pub from: String,
    pub sort_by: Option<String>,
    pub group_by: Option<String>,
    pub group_by_order: Option<String>,
    pub group_by_limit: Option<usize>,
    pub order: Option<String>,
    pub limit: Option<usize>,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TemplateContentDSLItem {
    Normal(Vec<ContentItem>),
    Grouped(IndexMap<String, Vec<ContentItem>>),
    Pulled(serde_json::Value),
}

/// Sort, order and limit given `items` according to given `dsl`.
pub fn dsl_sort_order_limit(dsl: ContentDSLItem, items: &mut Vec<ContentItem>) -> Vec<ContentItem> {
    // Sort and order?
    if dsl.sort_by.is_some() {
        super::utils::sort_content_items(
            items,
            dsl.sort_by.unwrap_or(String::from("slug")),
            dsl.order.unwrap_or(String::from("desc")),
        );
    }

    // Limit?
    if dsl.limit.is_some() {
        items.truncate(dsl.limit.unwrap());
    }

    return items.to_vec();
}

/// Returns a grouper from a given `item` according to given `by`. The
/// `by` can be any top-level struct key as well as meta-level key, such as
/// `meta.date`. In the case of `meta.date`, it also supports an additional
/// modifier such as `meta.date|year`, to group by year. `month` and `day`
/// are also supported.
pub fn dsl_group_by_grouper(item: &ContentItem, by: &String) -> String {
    let grouper: String;

    // Meta-key grouping.
    if by.contains("meta.") {
        let meta_key: String;

        // Construct key
        if by.contains("|") {
            let whole_key = by.replace("meta.", "");
            let meta_key_split: Vec<&str> = whole_key.split("|").collect();
            meta_key = meta_key_split[0].to_string();
        } else {
            meta_key = by.replace("meta.", "");
        }

        // Construct modifier
        let meta_modifier: String;

        if by.contains("|") {
            let whole_key = by.replace("meta.", "");
            let meta_key_split: Vec<&str> = whole_key.split("|").collect();
            meta_modifier = meta_key_split[1].to_string();
        } else {
            meta_modifier = String::new();
        };

        // Construct value
        let value;

        if item.meta.get(&meta_key).is_some() {
            value = item.meta.get(&meta_key).unwrap().to_string();
        } else {
            value = String::new();
        };

        // If we're grouping by meta.date and have `year` as a modifier
        if meta_key == "date" && meta_modifier == "year" {
            let date_parts: Vec<&str> = value.split("-").collect();
            grouper = date_parts[0].to_string();
        // If we're grouping by meta.date and have `month` as a modifier
        } else if meta_key == "date" && meta_modifier == "month" {
            let date_parts: Vec<&str> = value.split("-").collect();
            grouper = date_parts[1].to_string();
        // If we're grouping by meta.date and have `day` as a modifier
        } else if meta_key == "date" && meta_modifier == "day" {
            let date_parts: Vec<&str> = value.split("-").collect();
            grouper = date_parts[2].to_string();
        // Otherwise, the value itself is the grouper
        } else {
            grouper = value;
        }
    // Group by top-level field key.
    } else {
        grouper = super::utils::get_field_by_name(item, &by);
    }

    return grouper;
}

/// Order given `groups` in either a descending or ascending order. Given
/// `order` must either be a `asc` or `desc` string.
pub fn dsl_group_order_limit(
    groups: IndexMap<String, Vec<ContentItem>>,
    order: String,
    limit: Option<usize>,
) -> IndexMap<String, Vec<ContentItem>> {
    let mut ordered_grouped_content: IndexMap<String, Vec<ContentItem>> = IndexMap::new();
    let mut keys: Vec<String> = Vec::new();

    for key in groups.keys() {
        keys.push(key.to_string());
    }

    // Order
    keys.sort();

    if order == "desc" {
        keys.reverse();
    }

    // Limit
    if limit.is_some() {
        keys.truncate(limit.unwrap());
    }

    // Construct IndexMap
    for key in keys {
        let scoped_key = key.clone();
        ordered_grouped_content.insert(scoped_key, groups.get(&key).unwrap().to_vec());
    }

    return ordered_grouped_content;
}

/// Group given `items` by given `by` and, optionally, order the groups by
/// given `order`.
pub fn dsl_group(
    items: Vec<ContentItem>,
    by: String,
    order: Option<String>,
    limit: Option<usize>,
) -> IndexMap<String, Vec<ContentItem>> {
    // If by is not provided, return nothing. This is so that the
    // `compose_content_from_dsl` function would know which enum
    // to return, as in grouped or normal.
    if by.is_empty() {
        return IndexMap::new();
    }

    // Groups the items by a given grouper, which is a string
    // indicating a top-level struct key, or a meta key via "meta.{key}".
    let mut grouped_content: IndexMap<String, Vec<ContentItem>> = IndexMap::new();

    for item in items {
        let grouper = dsl_group_by_grouper(&item, &by);
        let mut grouped_content_items: Vec<ContentItem> = grouped_content
            .get(&grouper)
            .unwrap_or(&Vec::new())
            .to_vec();

        grouped_content_items.push(item);

        if grouped_content.get(&grouper).is_none() {
            grouped_content.insert(grouper, grouped_content_items);
        } else {
            grouped_content.remove(&grouper);
            grouped_content.insert(grouper, grouped_content_items);
        }
    }

    // Order the groups by either descending (default) or ascending order.
    if order.is_some() {
        grouped_content = dsl_group_order_limit(grouped_content, order.unwrap(), limit);
    }

    return grouped_content;
}
