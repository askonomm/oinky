use cached::proc_macro::cached;
use chrono::prelude::*;
use comrak::{markdown_to_html, ComrakOptions};
use dotenv::dotenv;
use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, Renderable};
use indexmap::IndexMap;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json;
use serde_value::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::prelude::*;
use std::path::Path;

enum FileType {
    Handlebars,
    HandlebarsPages,
    Markdown,
    Asset,
}

#[derive(Clone)]
struct TemplatePartial {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum TemplateContentDSLItem {
    Normal(Vec<ContentItem>),
    Grouped(IndexMap<String, Vec<ContentItem>>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemplateData {
    site: serde_json::Value,
    content: HashMap<String, TemplateContentDSLItem>,
    path: Option<String>,
    slug: Option<String>,
    meta: Option<HashMap<String, String>>,
    entry: Option<String>,
    time_to_read: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContentItem {
    path: String,
    slug: String,
    meta: HashMap<String, String>,
    entry: String,
    time_to_read: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContentDSLItem {
    name: String,
    from: String,
    sort_by: Option<String>,
    group_by: Option<String>,
    group_by_order: Option<String>,
    group_by_limit: Option<usize>,
    order: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone)]
struct Config {
    dir: String,
}

/// Prints an error `message` to stdout and subsequently exits the program.
fn err_out(message: String) {
    print!("{}", message);
    std::process::exit(1);
}

/// Returns runtime config for Oink such as the directory
/// where to run Oink in. If dotenv values for these exist then it will
/// use those instead.
#[cached]
fn get_config() -> Config {
    let dir;
    let env_dir = env::var("READ_DIR");

    if env_dir.is_ok() {
        dir = env_dir.unwrap().to_string();
    } else {
        dir = env::current_dir().unwrap().to_str().unwrap().to_string();
    }

    return Config { dir };
}

/// Recursively browses directories within the given `dir` for any and all
/// files that match a `file_type`. Returns a vector of strings where each
/// string is an absolute path to the file.
fn find_files(dir: &Path, file_type: &FileType) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let path_str = path.as_path().display().to_string();

        if path.is_dir() {
            files.extend(find_files(&path, file_type));
        }

        match file_type {
            FileType::Handlebars => {
                if path_str.ends_with(".hbs") || path_str.ends_with(".handlebars") {
                    files.push(path_str);
                }
            }
            FileType::HandlebarsPages => {
                if (!path_str.contains("_layouts") && !path_str.contains("_partials"))
                    && (path_str.ends_with(".hbs") || path_str.ends_with(".handlebars"))
                {
                    files.push(path_str);
                }
            }
            FileType::Markdown => {
                if path_str.ends_with(".md") || path_str.ends_with(".markdown") {
                    files.push(path_str);
                }
            }
            FileType::Asset => {
                if path_str.ends_with(".css")
                    || path_str.ends_with(".js")
                    || path_str.ends_with(".jpg")
                    || path_str.ends_with(".png")
                    || path_str.ends_with(".svg")
                    || path_str.ends_with(".ttf")
                    || path_str.ends_with(".woff")
                    || path_str.ends_with(".woff2")
                {
                    files.push(path_str);
                }
            }
        }
    }

    return files;
}

/// Finds all partials from within the /_partials directory that
/// it turns into a vector of consumable `TemplatePartial`'s. Consumed by
/// Handlebars in `built_html`.
#[cached(time = 2)]
fn find_partials() -> Vec<TemplatePartial> {
    let paths = find_files(
        Path::new(&format!("{}{}", get_config().dir, "/_partials")),
        &FileType::Handlebars,
    );
    let mut partials: Vec<TemplatePartial> = Vec::new();

    for path in paths {
        let partial_path_split: Vec<&str> = path.split("/").collect();
        let partial_name = partial_path_split
            .last()
            .copied()
            .unwrap()
            .replace(".hbs", "");

        let partial = TemplatePartial {
            name: partial_name,
            path: path,
        };

        partials.push(partial);
    }

    return partials;
}

/// Parses a given content item's `contents` for YAML-like meta-data which it
/// then returns as a key-value HashMap.
fn parse_content_file_meta(contents: &str) -> HashMap<String, String> {
    let regex = Regex::new(r"(?s)^(---)(.*?)(---|\.\.\.)").unwrap();
    let meta_block = regex.find(&contents).unwrap().as_str();
    let meta_lines = meta_block.lines();
    let mut meta: HashMap<String, String> = HashMap::new();

    for line in meta_lines {
        if line != "---" {
            let split_line: Vec<&str> = line.split(":").collect();
            let key = split_line[0].trim().to_string();
            let val = split_line[1].trim().to_string();

            meta.insert(key, val);
        }
    }

    return meta;
}

/// Parses a given content item's `contents` for the Markdown entry which it
/// then returns as a consumable HTML string.
fn parse_content_file_entry(contents: &str) -> String {
    let regex = Regex::new(r"(?s)^---(.*?)---*").unwrap();
    let entry = regex.replace(&contents, "");

    return markdown_to_html(&entry, &ComrakOptions::default());
}

/// Parses given Markdown `files` for contents that contain YAML-like meta-data
/// and the Markdown entry. Returns a vector of `ContentItem`.
fn parse_content_files(files: &Vec<String>) -> Vec<ContentItem> {
    let mut content_items: Vec<ContentItem> = Vec::new();

    for file in files {
        let file_contents = fs::read_to_string(file);
        let contents = file_contents.unwrap_or(String::new());
        let meta = parse_content_file_meta(&contents);
        let entry = String::new(); // parse_content_file_entry(&contents);
        let path = file.to_string();
        let slug = file
            .to_string()
            .replace(&get_config().dir, "")
            .replace(".md", "");
        let time_to_read = entry.split_whitespace().count() / 225;
        let content_item = ContentItem {
            path,
            slug,
            meta,
            entry,
            time_to_read,
        };

        content_items.push(content_item);
    }

    return content_items;
}

/// Handlebars date helper.
/// Usage:
///
/// ```handlebars
/// {{date "%Y %d %m"}}
/// ```
fn date_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    if !h.param(0).unwrap().is_value_missing() {
        let format: String = serde_json::from_value(h.param(0).unwrap().value().clone()).unwrap();
        let dt = Utc::now();
        let result = dt.format(&format).to_string();

        out.write(&result)?;
    }

    Ok(())
}

/// Handlebars date formatter helper.
/// Usage:
///
/// ```handlebars
/// {{format_date date-string "%Y %d %m"}}
/// ```
fn format_date_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    if !h.param(0).unwrap().is_value_missing() {
        let date: String = serde_json::from_value(h.param(0).unwrap().value().clone()).unwrap();
        let date_parts: Vec<&str> = date.split("-").collect();
        let year = date_parts[0].parse::<i32>().unwrap();
        let month = date_parts[1].parse::<u32>().unwrap();
        let day = date_parts[2].parse::<u32>().unwrap();
        let format: String = serde_json::from_value(h.param(1).unwrap().value().clone()).unwrap();
        let dt = Utc.ymd(year, month, day).and_hms(12, 0, 9);
        let result = dt.format(&format).to_string();

        out.write(&result)?;
    }

    Ok(())
}

/// Handlebars slug checking helper.
/// Usage:
///
/// ```handlebars
/// {{#is_slug "/archive/index.html"}}
/// // my code goes here
/// {{/is_slug}}
/// ```
fn is_slug_helper(
    h: &Helper,
    r: &Handlebars,
    c: &Context,
    rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let mut x = rc.clone();

    if !h.param(0).unwrap().is_value_missing() {
        let path: String = serde_json::from_value(h.param(0).unwrap().value().clone()).unwrap();
        let data: TemplateData = serde_json::from_value(c.data().clone()).unwrap();
        let slug = data.slug;

        if slug.is_some() && slug.unwrap() == path && h.template().is_some() {
            h.template().unwrap().render(&r, &c, &mut x, out).unwrap();
        }
    }

    Ok(())
}

/// Handlebars slug checking helper.
/// Usage:
///
/// ```handlebars
/// {{#unless_slug "/archive/index.html"}}
/// // my code goes here
/// {{/unless_slug}}
/// ```
fn unless_slug_helper(
    h: &Helper,
    r: &Handlebars,
    c: &Context,
    rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let mut x = rc.clone();

    if !h.param(0).unwrap().is_value_missing() {
        let path: String = serde_json::from_value(h.param(0).unwrap().value().clone()).unwrap();
        let data: TemplateData = serde_json::from_value(c.data().clone()).unwrap();
        let slug = data.slug;

        if slug.is_some() && slug.unwrap() != path && h.template().is_some() {
            h.template().unwrap().render(&r, &c, &mut x, out).unwrap();
        }
    }

    Ok(())
}

/// Builds HTML from a Handlebars template in a path `template_path`, by fusing
/// together `data` and registering any given `partials`. Returns a HTML string.
fn build_html(template_path: String, partials: Vec<TemplatePartial>, data: TemplateData) -> String {
    let mut hbs = Handlebars::new();

    // Register the main template
    let main_template = hbs.register_template_file("_main", &template_path);

    if main_template.is_err() {
        err_out(format!(
            "Something went wrong within your template, {}: {:?}",
            template_path,
            main_template.err()
        ));
    }

    // Register partials
    for partial in partials {
        let partial_template = hbs.register_template_file(&partial.name, partial.path);

        if partial_template.is_err() {
            err_out(format!(
                "Something went wrong within your partial, {}: {:?}",
                partial.name,
                partial_template.err()
            ));
        }
    }

    // Register helpers
    hbs.register_helper("date", Box::new(date_helper));
    hbs.register_helper("format_date", Box::new(format_date_helper));
    hbs.register_helper("is_slug", Box::new(is_slug_helper));
    hbs.register_helper("unless_slug", Box::new(unless_slug_helper));

    // Render
    let render = hbs.render("_main", &data);

    if render.is_ok() {
        return render.unwrap();
    } else {
        err_out(format!("There seems to be an error: {:?}", render.err()));
        return String::new();
    }
}

/// Deletes all files and directories from within the /public directory.
fn empty_public_dir() {
    let config = get_config();
    let path = &format!("{}{}", config.dir, "/public");

    if fs::read_dir(path).is_err() {
        return;
    }

    for entry in fs::read_dir(path).unwrap() {
        let file = entry.unwrap();
        let file_path_str = file.path().as_path().display().to_string();

        if file.path().is_dir() {
            let remove_dir = fs::remove_dir_all(file.path());

            if remove_dir.is_err() {
                err_out(format!("Could not remove dir {}", file_path_str));
            }
        } else {
            let remove_file = fs::remove_file(file.path());

            if remove_file.is_err() {
                err_out(format!("Could not remove file {}", file_path_str));
            }
        }
    }
}

/// Writes given `contents` into given `path. Parent directories do not have
/// to exist as they will also be created if they don't.
fn write_to_path(path: &str, contents: String) {
    let path = Path::new(&path);
    let prefix = path.parent().unwrap();
    fs::create_dir_all(prefix).unwrap();

    let mut file = fs::File::create(path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file.sync_data().unwrap();
}

/// Compiles all content items within the root directory with given
/// global Handlebars `data`, resulting in HTML files written to disk.
fn compile_content_items(data: &TemplateData) {
    let config = get_config();
    let read_path = Path::new(&config.dir);
    let content_files = find_files(read_path, &FileType::Markdown);
    let content_items = parse_content_files(&content_files);
    let partials = find_partials();

    for content_item in content_items {
        println!("Building {}", content_item.slug);

        let item = content_item.clone();
        let item_data = TemplateData {
            path: Some(content_item.path),
            slug: Some(content_item.slug),
            meta: Some(content_item.meta),
            entry: Some(content_item.entry),
            time_to_read: Some(content_item.time_to_read),
            ..data.clone()
        };

        let layout: String;

        if item.meta.get("layout").is_some() {
            layout = item.meta.get("layout").unwrap().to_string();
        } else {
            layout = String::from("default");
        }

        let template_path = format!("{}{}{}{}", get_config().dir, "/_layouts/", layout, ".hbs");
        let html = build_html(template_path, partials.clone(), item_data);
        let write_path = format!(
            "{}{}{}{}",
            get_config().dir,
            "/public",
            item.slug,
            "/index.html"
        );

        write_to_path(&write_path, html);
    }
}

/// Compiles all non-layout and non-partial template items within the
/// root directory with given Handlebars `data`, resulting in HTML files
/// written to disk.
fn compile_template_items(data: &TemplateData) {
    let config = get_config();
    let read_path = Path::new(&config.dir);
    let partials = find_partials();
    let template_files = find_files(read_path, &FileType::HandlebarsPages);

    for file in template_files {
        let slug = file
            .to_string()
            .replace(&config.dir, "")
            .replace(".hbs", "");

        println!("Building {}", slug);

        let template_data = TemplateData {
            slug: Some(slug.clone()),
            ..data.clone()
        };

        let html = build_html(file, partials.clone(), template_data);
        let write_path = format!("{}{}{}", get_config().dir, "/public", slug);

        write_to_path(&write_path, html);
    }
}

/// Returns a value of a given `s` by a given `field`. Enables the retrieval
/// of Struct values by key using a string.
fn get_field_by_name<T, R>(s: T, field: &str) -> R
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
        None => panic!("{}", format!("no such field {:?}", key)),
    };

    match R::deserialize(value) {
        Ok(r) => r,
        Err(_) => panic!("Something went wrong ..."),
    }
}

/// Sorts given `items` by given `by` in given `order`. Supports top-level struct
/// keys as `by` as well as meta-level keys like `meta.date`.
fn sort_content_items(items: &mut Vec<ContentItem>, by: String, order: String) {
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

/// Sort, order and limit given `items` according to given `dsl`.
fn dsl_sort_order_limit(dsl: ContentDSLItem, items: &mut Vec<ContentItem>) -> Vec<ContentItem> {
    // Sort and order?
    if dsl.sort_by.is_some() {
        sort_content_items(
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
        grouper = get_field_by_name(item, &by);
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

/// Composes content data from the `content.json` DSL which allows users to
/// create data-sets from the available content files, further enabling more
/// dynamic-ish site creation.
fn compose_content_from_dsl() -> HashMap<String, TemplateContentDSLItem> {
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
        let path_str = format!("{}{}{}", config.dir, "/", dsl_item.from);
        let content_files = find_files(Path::new(&path_str), &FileType::Markdown);
        let mut parsed_content_files = parse_content_files(&content_files);

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

/// Composes global template data for consumption by Handlebars templates.
fn compose_global_template_data() -> TemplateData {
    return TemplateData {
        site: get_site_info(),
        content: compose_content_from_dsl(),
        path: None,
        slug: None,
        meta: None,
        entry: None,
        time_to_read: None,
    };
}

/// Return `SiteInfo` from the `site.json` file.
#[cached(time = 2)]
fn get_site_info() -> serde_json::Value {
    let config = get_config();
    let file_contents = fs::read_to_string(format!("{}{}", config.dir, "/site.json"));
    let contents = file_contents.unwrap_or(String::new());

    return serde_json::from_str(&contents).unwrap();
}

/// Copies all files with `FileType::Asset` into the /public directory.
fn copy_assets() {
    let config = get_config();
    let assets = find_files(Path::new(&config.dir), &FileType::Asset);

    for asset in assets {
        let relative_path = asset.replace(&config.dir, "");
        println!("Copying {}", relative_path);
        let action = fs::copy(
            asset,
            format!("{}{}{}", config.dir, "/public", relative_path),
        );

        if action.is_err() {
            err_out(format!("Could not copy file {}", relative_path));
        }
    }
}

fn main() {
    // Prepare dotenv
    dotenv().ok();

    // Empty the public dir
    empty_public_dir();

    // Construct global Handlebars data
    let global_data = compose_global_template_data();

    // Compile individual content items
    compile_content_items(&global_data);

    // Compile individual non-layout and non-partial Handlebars templates.
    compile_template_items(&global_data);

    // Move assets to /public dir
    copy_assets();
}
