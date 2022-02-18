use super::{find_files, get_config, parse_content_files, ContentItem, FileType};
use cached::proc_macro::cached;
use indexmap::IndexMap;
use isahc::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

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
    Single(ContentItem),
    Pulled(serde_json::Value),
}

/// Sort, order and limit given `items` according to given `dsl`.
fn dsl_sort_order_limit(dsl: ContentDSLItem, items: &mut Vec<ContentItem>) -> Vec<ContentItem> {
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
fn dsl_group_by_grouper(item: &ContentItem, by: &String) -> String {
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
fn dsl_group_order_limit(
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
fn dsl_group(
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

fn get_content_from_http(from: String) -> Option<TemplateContentDSLItem> {
    let client = isahc::HttpClient::builder()
        .default_headers(dsl_item.headers.unwrap_or(HashMap::new()))
        .build()
        .unwrap();

    let response = client.get(dsl_item.from);

    if response.is_ok() {
        return Some(TemplateContentDSLItem::Pulled(
            serde_json::from_str(&response.unwrap().text().unwrap()).unwrap(),
        ));
    }

    println!("{:#?}", response.err());
    return None;
}

/// Composes content data from the `content.json` DSL which allows users to
/// create data-sets from the available content files, further enabling more
/// dynamic-ish site creation.
#[cached(time = 2)]
pub fn compose_content_from_dsl() -> HashMap<String, TemplateContentDSLItem> {
    let config = get_config();
    let file_contents = fs::read_to_string(format!("{}{}", config.dir, "/content.json"));
    let contents = file_contents.unwrap_or_default();
    let dsl: Result<Vec<ContentDSLItem>, serde_json::Error> = serde_json::from_str(&contents);

    if dsl.is_err() {
        return HashMap::new();
    }

    let mut content: HashMap<String, TemplateContentDSLItem> = HashMap::new();

    for dsl_item in dsl.unwrap_or(Vec::new()) {
        let item = dsl_item.clone();

        // HTTP fetched data
        if dsl_item.from.starts_with("http") {
            let pulled_content = pull_content_from_http(dsl_item.from);

            if pulled_content.is_some() {
                content.insert(
                    dsl_item.name,
                    pulled_content.unwrap(),
                );
            }

            continue;
        }

        // Markdown data
        let path_str = format!("{}{}{}", config.dir, "/", dsl_item.from);
        let single_item = path_str.ends_with(".md") || path_str.ends_with(".markdown");
        let mut content_files: Vec<String> = Vec::new();

        if single_item {
            content_files.push(path_str);
        } else {
            content_files = find_files(path_str, FileType::Markdown);
        }

        let mut parsed_content_files = parse_content_files(content_files);

        if single_item && parsed_content_files.len() > 0 {
            content.insert(
                dsl_item.name,
                TemplateContentDSLItem::Single(parsed_content_files.first().unwrap().clone()),
            );

            continue;
        }

        if dsl_item.group_by.is_some() {
            content.insert(
                dsl_item.name,
                TemplateContentDSLItem::Grouped(dsl_group(
                    dsl_sort_order_limit(item, &mut parsed_content_files),
                    dsl_item.group_by.unwrap(),
                    dsl_item.group_by_order,
                    dsl_item.group_by_limit,
                )),
            );
        } else {
            content.insert(
                dsl_item.name,
                TemplateContentDSLItem::Normal(dsl_sort_order_limit(
                    item,
                    &mut parsed_content_files,
                )),
            );
        }
    }

    return content;
}
