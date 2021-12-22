use cached::proc_macro::cached;
use comrak::{markdown_to_html, ComrakOptions};
use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_value::Value;
use std::collections::HashMap;
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

#[derive(Debug, Clone, Serialize)]
struct TemplateData {
    content: HashMap<String, Vec<ContentItem>>,
    path: Option<String>,
    slug: Option<String>,
    meta: Option<HashMap<String, String>>,
    entry: Option<String>,
    time_to_read: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct ContentItem {
    path: String,
    slug: String,
    meta: HashMap<String, String>,
    entry: String,
    time_to_read: usize,
}

#[derive(Clone, Serialize, Deserialize)]
struct ContentDSLItem {
    name: String,
    from: String,
    sort_by: Option<String>,
    order: Option<String>,
    limit: Option<usize>,
    group_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SiteInfoItem {
    name: String,
    value: String,
}

/// Prints an error `message` to stdout and subsequently exits the program.
fn err_out(message: String) {
    print!("{}", message);
    std::process::exit(1);
}

#[cached]
fn get_dir() -> &'static str {
    //let current_dir = std::env::current_dir().unwrap_or(Path::new("./").to_path_buf());
    const READ_DIR: &str = "../bien.ee";

    return READ_DIR;
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
        Path::new(&format!("{}{}", get_dir(), "/_partials")),
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
        let entry = parse_content_file_entry(&contents);
        let path = file.to_string();
        let slug = file.to_string().replace(get_dir(), "").replace(".md", "");
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

fn site_info_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let first_param = h.param(0).unwrap();
    let site_info = get_site_info();
    let key: Result<String, serde_json::Error> =
        serde_json::from_value(first_param.value().clone());
    let val = site_info.get(&key.unwrap());

    if val.is_some() {
        out.write(val.unwrap())?;
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
    hbs.register_helper("site", Box::new(site_info_helper));

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
    let path = &format!("{}{}", get_dir(), "/public");

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
/// exist as it will also create them itself if they don't exist.
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
    let read_path = Path::new(get_dir());
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

        let template_path = format!(
            "{}{}{}{}",
            get_dir(),
            "/_layouts/",
            item.meta["layout"].as_str().to_string(),
            ".hbs"
        );

        let html = build_html(template_path, partials.clone(), item_data);
        let write_path = format!("{}{}{}{}", get_dir(), "/public", item.slug, "/index.html");

        write_to_path(&write_path, html);
    }
}

/// Compiles all non-layout and non-partial template items within the 
/// root directory with given Handlebars `data`, resulting in HTML files 
/// written to disk.
fn compile_template_items(data: &TemplateData) {
    let read_path = Path::new(get_dir());
    let partials = find_partials();
    let template_files = find_files(read_path, &FileType::HandlebarsPages);

    for file in template_files {
        let slug = file.to_string().replace(get_dir(), "").replace(".hbs", "");
        println!("Building {}", slug);

        let html = build_html(file, partials.clone(), data.clone());
        let write_path = format!("{}{}{}", get_dir(), "/public", slug);

        write_to_path(&write_path, html);
    }
}

fn get_field_by_name<T, R>(data: T, field: &str) -> R
where
    T: Serialize,
    R: DeserializeOwned,
{
    let mut map = match serde_value::to_value(data) {
        Ok(Value::Map(map)) => map,
        _ => panic!("expected a struct"),
    };

    let key = Value::String(field.to_owned());
    println!("{:?}", key);
    let value = match map.remove(&key) {
        Some(value) => value,
        None => panic!("no such field"),
    };

    match R::deserialize(value) {
        Ok(r) => r,
        Err(_) => panic!("wrong type?"),
    }
}

fn sort_content_items(content_items: &mut Vec<ContentItem>, by: String, order: String) {
    content_items.sort_by(|a, b| {
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

fn blah(dsl: ContentDSLItem, content_items: &mut Vec<ContentItem>) -> Vec<ContentItem> {
    // Sort and order?
    if dsl.sort_by.is_some() {
        let mut order = String::from("desc");

        if dsl.order.is_some() {
            order = dsl.order.unwrap();
        }

        sort_content_items(content_items, dsl.sort_by.unwrap(), order);
    }

    // Group by?
    if dsl.group_by.is_some() {
        let group_by = dsl.group_by.unwrap();

        let group_by_split: Vec<&str> = group_by.split("|").collect();
    }

    // Limit?
    if dsl.limit.is_some() {
        content_items.truncate(dsl.limit.unwrap());
    }

    return content_items.to_vec();
}

/// Composes content data from the `content.json` DSL which allows users to
/// create data-sets from the available content files, further enabling more
/// dynamic-ish site creation.
fn compose_content_from_dsl() -> HashMap<String, Vec<ContentItem>> {
    let file_contents = fs::read_to_string(format!("{}{}", get_dir(), "/content.json"));
    let contents = file_contents.unwrap_or_default();
    let dsl: Result<Vec<ContentDSLItem>, serde_json::Error> = serde_json::from_str(&contents);

    if dsl.is_err() {
        return HashMap::new();
    }

    let mut content: HashMap<String, Vec<ContentItem>> = HashMap::new();

    for dsl_item in dsl.unwrap_or(Vec::new()) {
        let path_str = format!("{}{}{}", get_dir(), "/", dsl_item.from);
        let content_files = find_files(Path::new(&path_str), &FileType::Markdown);
        let content_items = blah(dsl_item.clone(), &mut parse_content_files(&content_files));

        content.insert(dsl_item.name, content_items);
    }

    return content;
}

/// Composes global template data for consumption by Handlebars templates.
fn compose_global_template_data() -> TemplateData {
    return TemplateData {
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
fn get_site_info() -> HashMap<String, String> {
    println!("Reading site info ...");
    let file_contents = fs::read_to_string(format!("{}{}", get_dir(), "/site.json"));
    let contents = file_contents.unwrap_or(String::new());
    let data: Result<Vec<SiteInfoItem>, serde_json::Error> = serde_json::from_str(&contents);

    if data.is_err() {
        err_out("Could not read site info from site.json.".to_string());

        return HashMap::new();
    }

    let mut site_info = HashMap::new();

    for data_item in data.unwrap_or(Vec::new()) {
        site_info.insert(data_item.name, data_item.value);
    }

    return site_info;
}

/// Copies all files with `FileType::Asset` into the /public directory.
fn copy_assets() {
    let assets = find_files(Path::new(get_dir()), &FileType::Asset);

    for asset in assets {
        let relative_path = asset.replace(get_dir(), "");
        println!("Copying {}", relative_path);
        let action = fs::copy(
            asset,
            format!("{}{}{}", get_dir(), "/public", relative_path),
        );

        if action.is_err() {
            err_out(format!("Could not copy file {}", relative_path));
        }
    }
}

fn main() {
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
