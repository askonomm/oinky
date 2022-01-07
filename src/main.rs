use cached::proc_macro::cached;
use chrono::prelude::*;
use comrak::{markdown_to_html, ComrakOptions};
use dotenv::dotenv;
use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, Renderable};
use hotwatch::{Event, Hotwatch};
use indexmap::IndexMap;
use isahc::prelude::*;
use parking_lot;
use rayon::prelude::*;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json;
use serde_value::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use throttle_my_fn::throttle;

#[derive(Clone, Eq, PartialEq, Hash)]
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
    Pulled(serde_json::Value),
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
    headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
struct Config {
    dir: String,
    utc_offset: i32,
}

/// Prints an error `message` to stdout and subsequently exits the program.
fn err_out(message: String) {
    println!("{}", message);
    std::process::exit(1);
}

/// Returns runtime config for Oinky such as the directory
/// where to run Oinky in. If dotenv values for these exist then it will
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

    let mut utc_offset = 0;
    let env_utc_offset = env::var("UTC_OFFSET");

    if env_utc_offset.is_ok() {
        utc_offset = env_utc_offset.unwrap().parse::<i32>().unwrap();
    }

    return Config { dir, utc_offset };
}

/// Determines if the given `path` matches a Handlebars file.
fn is_handlebars_file(path: &str) -> bool {
    return path.ends_with(".hbs") || path.ends_with(".handlebars");
}

/// Determines if the given `path` matches a Handlebars Page file.
fn is_handlebars_page_file(path: &str) -> bool {
    return !path.contains("_layouts")
        && !path.contains("_partials")
        && !path.contains("public")
        && !path.contains("node_modules")
        && (path.ends_with(".hbs") || path.ends_with(".handlebars"));
}

/// Determines if the given `path` matches a Markdown file.
fn is_markdown_file(path: &str) -> bool {
    return !path.contains("_layouts")
        && !path.contains("_partials")
        && !path.contains("public")
        && !path.contains("node_modules")
        && (path.ends_with(".md") || path.ends_with(".markdown"));
}

/// Determines if the given `path` matches a data file.
fn is_data_file(path: &str) -> bool {
    let relative_path = path.replace(&get_config().dir, "");

    return relative_path == "/site.json" || relative_path == "/content.json";
}

/// Determines if the given `path` matches a asset file.
fn is_asset_file(path: &str) -> bool {
    let relative_path = path.replace(&get_config().dir, "");

    return !path.ends_with(".hbs")
        && !path.ends_with(".handlebars")
        && !path.ends_with(".md")
        && !path.ends_with(".markdown")
        && !path.ends_with("site.json")
        && !path.ends_with("content.json")
        && !path.contains("_layouts")
        && !path.contains("_partials")
        && !path.contains("node_modules")
        && !path.contains("public")
        && !relative_path.starts_with("/.");
}

/// Recursively browses directories within the given `dir` for any and all
/// files that match a `file_type`. Returns a vector of strings where each
/// string is an absolute path to the file.
#[cached]
fn find_files(dir: String, file_type: FileType) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let read_dir = fs::read_dir(dir);

    if read_dir.is_err() {
        return Vec::new();
    }

    for entry in read_dir.unwrap() {
        let path = entry.unwrap().path();
        let path_str = path.as_path().display().to_string();

        if path.is_dir() {
            files.extend(find_files(path_str.clone(), file_type.clone()));
        } else {
            match file_type {
                FileType::Handlebars => {
                    if is_handlebars_file(&path_str) {
                        files.push(path_str);
                    }
                }
                FileType::HandlebarsPages => {
                    if is_handlebars_page_file(&path_str) {
                        files.push(path_str);
                    }
                }
                FileType::Markdown => {
                    if is_markdown_file(&path_str) {
                        files.push(path_str);
                    }
                }
                FileType::Asset => {
                    if is_asset_file(&path_str) {
                        files.push(path_str);
                    }
                }
            }
        }
    }

    return files;
}

/// Finds all partials from within the /_partials directory that
/// it turns into a vector of consumable `TemplatePartial`'s. Consumed by
/// Handlebars in `build_html`.
#[cached]
fn find_partials() -> Vec<TemplatePartial> {
    return find_files(
        format!("{}{}", get_config().dir, "/_partials"),
        FileType::Handlebars,
    )
    .par_iter()
    .map(|path| {
        let partial_path_split: Vec<&str> = path.split("/").collect();
        let partial_name = partial_path_split
            .last()
            .copied()
            .unwrap()
            .replace(".hbs", "");

        return TemplatePartial {
            name: partial_name,
            path: path.clone(),
        };
    })
    .collect();
}

/// Parses a given content item's `contents` for YAML-like meta-data which it
/// then returns as a key-value HashMap.
#[cached]
fn parse_content_file_meta(contents: String) -> HashMap<String, String> {
    let regex = Regex::new(r"(?s)^(---)(.*?)(---|\.\.\.)").unwrap();

    if regex.find(&contents).is_none() {
        return HashMap::new();
    }

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
#[cached]
fn parse_content_file_entry(contents: String) -> String {
    let regex = Regex::new(r"(?s)^---(.*?)---*").unwrap();
    let entry = regex.replace(&contents, "");

    return markdown_to_html(&entry, &ComrakOptions::default());
}

/// Parses given Markdown `files` for contents that contain YAML-like meta-data
/// and the Markdown entry. Returns a vector of `ContentItem`.
#[cached]
fn parse_content_files(files: Vec<String>) -> Vec<ContentItem> {
    return files
        .par_iter()
        .map(|file| {
            let f = fs::File::open(file.clone()).expect("Could not read file.");
            let mut reader = BufReader::new(f);
            let mut contents = String::new();

            reader.read_to_string(&mut contents).unwrap();

            let meta = parse_content_file_meta(contents.clone());
            let entry = parse_content_file_entry(contents);
            let slug = file.replace(&get_config().dir, "").replace(".md", "");
            let time_to_read = entry.split_whitespace().count() / 225;
            return ContentItem {
                path: file.clone(),
                slug,
                meta,
                entry,
                time_to_read,
            };
        })
        .collect();
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
        let config = get_config();
        let hours = config.utc_offset;
        let offset = FixedOffset::east_opt(hours * 60 * 60)
            .expect("UTC offset out of bound, min -12, max 12");
        let dt = Utc::now().with_timezone(&offset);
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
        let config = get_config();
        let hours = config.utc_offset;
        let offset = FixedOffset::east_opt(hours * 60 * 60)
            .expect("UTC offset out of bound, min -12, max 12");
        let dt = Utc.ymd(year, month, day).with_timezone(&offset);
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
        let regex = Regex::new(&path);

        if (regex.is_err() || slug.is_none()) && h.inverse().is_some() {
            h.inverse().unwrap();
        }

        if slug.is_some() && regex.unwrap().is_match(&slug.unwrap()) && h.template().is_some() {
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
        let regex = Regex::new(&path);

        if regex.is_err() || slug.is_none() {
            h.inverse().unwrap();
        }

        if slug.is_some() && !regex.unwrap().is_match(&slug.unwrap()) && h.template().is_some() {
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

    let file = fs::File::create(path).unwrap();
    let mut file = BufWriter::new(file);
    file.write_all(contents.as_bytes()).unwrap();
    //file.sync_data().unwrap();
}

/// Compiles all content items within the root directory with given
/// global Handlebars `data`, resulting in HTML files written to disk.
fn compile_content_items(data: TemplateData) {
    let content_files = find_files(get_config().dir, FileType::Markdown);
    let content_items = parse_content_files(content_files);
    let chunks = content_items.chunks(500).map(|c| c.to_owned());
    static THREADS: AtomicUsize = AtomicUsize::new(0);

    for chunk in chunks {
        let x_data = data.clone();
        THREADS.fetch_add(1, Ordering::SeqCst);

        thread::spawn(move || {
            let x: Vec<ContentItem> = chunk;
            for content_item in x {
                if content_item.meta.get("layout").is_none() {
                    continue;
                }

                let item_data = TemplateData {
                    path: Some(content_item.path.clone()),
                    slug: Some(content_item.slug.clone()),
                    meta: Some(content_item.meta.clone()),
                    entry: Some(content_item.entry.clone()),
                    time_to_read: Some(content_item.time_to_read.clone()),
                    ..x_data.clone()
                };

                println!("Building {}", content_item.slug);

                let layout = content_item.meta.get("layout").unwrap().to_string();
                let template_path =
                    format!("{}{}{}{}", get_config().dir, "/_layouts/", layout, ".hbs");
                let html = build_html(template_path, find_partials(), item_data);
                let write_path = format!(
                    "{}{}{}{}",
                    get_config().dir,
                    "/public",
                    content_item.slug,
                    "/index.html"
                );

                write_to_path(&write_path, html);
            }

            THREADS.fetch_sub(1, Ordering::SeqCst);
        });
    }

    while THREADS.load(Ordering::SeqCst) != 0 {
        thread::sleep(Duration::from_millis(1));
    }
}

/// Compiles all non-layout and non-partial template items within the
/// root directory with given Handlebars `data`, resulting in HTML files
/// written to disk.
fn compile_template_items(data: TemplateData) {
    let template_files = find_files(get_config().dir, FileType::HandlebarsPages);
    let chunks = template_files.chunks(500).map(|c| c.to_owned());
    static THREADS: AtomicUsize = AtomicUsize::new(0);

    for chunk in chunks {
        let x_data = data.clone();
        THREADS.fetch_add(1, Ordering::SeqCst);

        thread::spawn(move || {
            for file in chunk {
                let slug = file
                    .to_string()
                    .replace(&get_config().dir, "")
                    .replace(".hbs", "");

                println!("Building {}", slug);

                let template_data = TemplateData {
                    slug: Some(slug.clone()),
                    ..x_data.clone()
                };

                let html = build_html(file, find_partials(), template_data);
                let write_path = format!("{}{}{}", get_config().dir, "/public", slug);

                write_to_path(&write_path, html);
            }

            THREADS.fetch_sub(1, Ordering::SeqCst);
        });
    }

    while THREADS.load(Ordering::SeqCst) != 0 {
        thread::sleep(Duration::from_millis(1));
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
#[cached]
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

        if dsl_item.from.starts_with("http") {
            let client = isahc::HttpClient::builder()
                .default_headers(dsl_item.headers.unwrap_or(HashMap::new()))
                .build()
                .unwrap();

            let response = client.get(dsl_item.from);

            if response.is_ok() {
                content.insert(
                    dsl_item.name,
                    TemplateContentDSLItem::Pulled(
                        serde_json::from_str(&response.unwrap().text().unwrap()).unwrap(),
                    ),
                );
            } else {
                println!("{:#?}", response.err());
            }

            continue;
        }

        let path_str = format!("{}{}{}", config.dir, "/", dsl_item.from);
        let content_files = find_files(path_str, FileType::Markdown);
        let mut parsed_content_files = parse_content_files(content_files);

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
#[cached]
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
#[cached]
fn get_site_info() -> serde_json::Value {
    let config = get_config();
    let file_contents = fs::read_to_string(format!("{}{}", config.dir, "/site.json"));
    let contents = file_contents.unwrap_or(String::new());

    return serde_json::from_str(&contents).unwrap_or(serde_json::from_str("{}").unwrap());
}

/// Deletes all `FileType::Asset` files from the `/public` directory.
fn delete_assets() {
    let assets = find_files(get_config().dir, FileType::Asset);

    for asset in assets {
        let relative_path = asset.replace(&get_config().dir, "");
        let public_dir_path = format!("{}{}{}", &get_config().dir, "/public", relative_path);
        let delete = fs::remove_file(public_dir_path);

        if delete.is_err() {
            println!("{:?}", delete.err());
        }
    }
}

/// Copies all `FileType::Asset` files into the /public directory.
fn copy_assets() {
    let assets = find_files(get_config().dir, FileType::Asset);

    for asset in assets {
        let relative_path = asset.replace(&get_config().dir, "");
        println!("Copying {}", relative_path);

        let full_new_path_str = format!("{}{}{}", &get_config().dir, "/public", relative_path);
        let path = Path::new(&full_new_path_str);
        let prefix = path.parent().unwrap();
        let create_dir = fs::create_dir_all(prefix);

        if create_dir.is_err() {
            println!("{:?}", create_dir.err());
        }

        let action = fs::copy(
            asset,
            format!("{}{}{}", get_config().dir, "/public", relative_path),
        );

        if action.is_err() {
            err_out(format!("Could not copy file {}", relative_path));
        }
    }
}

/// Runs Oinky on the current directory and compiles an entire static site
/// out of given information.
fn compile() {
    println!("Thinking ...");

    // Prepare dotenv
    dotenv().ok();

    // Empty the public dir
    empty_public_dir();

    // Construct global Handlebars data
    let global_data = compose_global_template_data();

    // Compile individual content items
    compile_content_items(global_data.clone());

    // Compile individual non-layout and non-partial Handlebars templates.
    compile_template_items(global_data.clone());

    // Move assets to /public dir
    copy_assets();
}

/// Potentially runs Oinky when a given `path` is determined to be something
/// that changes that would require the site generator to run again. Used by
/// the watcher.
#[throttle(1, Duration::from_secs(1))]
fn potentially_compile(path: PathBuf) {
    let path_str = path.as_path().display().to_string();

    // If data file or partials/layouts changed, re-compile everything
    if is_data_file(&path_str) || is_handlebars_file(&path_str) {
        compile()
    }

    // If assets changed, we need to delete all assets, and copy anew
    if is_asset_file(&path_str) {
        delete_assets();
        copy_assets();
    }

    // If content items changed, re-compile only those
    if is_markdown_file(&path_str) {
        let global_data = compose_global_template_data();

        compile_content_items(global_data.clone());
        compile_template_items(global_data);
    }

    // If template items changed, re-compile only those
    if is_handlebars_page_file(&path_str) {
        let global_data = compose_global_template_data();

        compile_template_items(global_data);
    }
}

/// Watches for file changes and potentally runs Oinky if an interesting enough
/// file has been created, changed, renamed or deleted.
fn watch() {
    let mut h = Hotwatch::new().expect("Watcher failed to initialize.");

    h.watch(get_config().dir, |event: Event| match event {
        Event::Write(path) => potentially_compile(path).unwrap_or(()),
        Event::Create(path) => potentially_compile(path).unwrap_or(()),
        Event::Rename(_, path) => potentially_compile(path).unwrap_or(()),
        Event::Remove(path) => potentially_compile(path).unwrap_or(()),
        _ => (),
    })
    .expect("Failed to watch directory.");

    thread::park();
}

fn main() {
    // Run Oinky
    compile();

    let args: Vec<String> = env::args().collect();

    // Potentially run a watcher
    if args.contains(&String::from("watch")) {
        watch();
    }
}
