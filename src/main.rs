use comrak::{markdown_to_html, ComrakOptions};
use handlebars::Handlebars;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

enum FileType {
    Handlebars,
    Markdown,
    Asset,
}

#[derive(Clone)]
struct TemplatePartial {
    name: String,
    path: String,
}

#[derive(Clone, Serialize)]
struct TemplateData {
    site: SiteInfo,
    current: Option<ContentItem>,
    content: HashMap<String, Vec<ContentItem>>,
}

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize, Deserialize)]
struct SiteInfoMetaItem {
    name: String,
    value: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct SiteInfo {
    title: Option<String>,
    url: Option<String>,
    meta: Option<Vec<SiteInfoMetaItem>>,
}

/// Prints an error `message` to stdout and subsequently exits the program.
fn err_out(message: String) {
    print!("{}", message);
    std::process::exit(1);
}

/// Recursively browsers directories within the given `dir` for any and all
/// files that match a `file_type`. Returns a vector of strings where each
/// string is an absolute path to the file.
fn find_files(dir: &Path, file_type: &FileType) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    if dir.is_dir() {
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
    }

    return files;
}

/// Finds all partials from within the `root_dir`/_partials directory that
/// it turns into a vector of consumable `TemplatePartial`'s. Consumed by
/// Handlebars in `built_html`.
fn find_partials(root_dir: &str) -> Vec<TemplatePartial> {
    let paths = find_files(
        Path::new(&format!("{}{}", root_dir, "/_partials")),
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
/// and the Markdown entry. It requires a `root_dir` to be passed so that it could
/// create relative URL's for each conten item (slugs). Returns a vector of
/// `ContentItem`.
fn parse_content_files(root_dir: &str, files: &Vec<String>) -> Vec<ContentItem> {
    let mut content_items: Vec<ContentItem> = Vec::new();

    for file in files {
        let file_contents = fs::read_to_string(file);
        let contents = file_contents.unwrap_or_default();
        let meta = parse_content_file_meta(&contents);
        let entry = parse_content_file_entry(&contents);
        let path = file.to_string();
        let slug = file.to_string().replace(root_dir, "").replace(".md", "");
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

    let render = hbs.render("_main", &data);

    if render.is_ok() {
        return render.unwrap();
    } else {
        err_out(format!("There seems to be an error: {:?}", render.err()));
        return String::new();
    }
}

/// Deletes all files and directories from within the /public directory inside of
/// the given `root_dir` directory.
fn empty_public_dir(root_dir: &str) {
    let path = &format!("{}{}", &root_dir, "/public");

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

    let mut file = File::create(path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file.sync_data().unwrap();
}

/// Compiles all content items within the `root_dir` directory with given 
/// global Handlebars `data`, resulting in HTML files written to disk. 
fn compile_content_items(root_dir: &str, data: &TemplateData) {
    let read_path = Path::new(root_dir);
    let content_files = find_files(read_path, &FileType::Markdown);
    let content_items = parse_content_files(root_dir, &content_files);
    let partials = find_partials(root_dir);

    for content_item in content_items {
        println!("Building {}", content_item.slug);

        let item = content_item.clone();
        let item_data = TemplateData {
            current: Some(content_item),
            ..data.clone()
        };

        let template_path = format!(
            "{}{}{}{}",
            root_dir,
            "/_layouts/",
            item.meta["layout"].as_str().to_string(),
            ".hbs"
        );

        let html = build_html(template_path, partials.clone(), item_data);
        let write_path = format!("{}{}{}{}", root_dir, "/public", item.slug, "/index.html");
        write_to_path(&write_path, html);
    }
}

/// Composes content data from the `content.json` DSL which allows users to 
/// create data-sets from the available content files, further enabling more 
/// dynamic-ish site creation. 
fn compose_content_from_dsl(root_dir: &str) -> HashMap<String, Vec<ContentItem>> {
    let file_contents = fs::read_to_string(format!("{}{}", root_dir, "/content.json"));
    let contents = file_contents.unwrap_or_default();
    let dsl: Result<Vec<ContentDSLItem>, serde_json::Error> = serde_json::from_str(&contents);

    if dsl.is_err() {
        return HashMap::new();
    }

    return HashMap::new();
}

/// Composes global template data for consumption by Handlebars templates.
fn compose_global_template_data(root_dir: &str) -> TemplateData {
    return TemplateData {
        site: get_site_info(&root_dir),
        current: None,
        content: compose_content_from_dsl(&root_dir),
    };
}

/// Return `SiteInfo` from the `site.json` file.
fn get_site_info(root_dir: &str) -> SiteInfo {
    let file_contents = fs::read_to_string(format!("{}{}", root_dir, "/site.json"));
    let contents = file_contents.unwrap_or_default();
    let data = serde_json::from_str(&contents);

    if data.is_err() {
        err_out("Could not read site info from site.json.".to_string());

        return SiteInfo {
            title: None,
            url: None,
            meta: None,
        };
    }

    return data.unwrap();
}

/// Copies all files with `FileType::Asset` into the /public directory.
fn copy_assets(root_dir: &str) {
    let assets = find_files(Path::new(root_dir), &FileType::Asset);

    for asset in assets {
        let relative_path = asset.replace(root_dir, "");
        println!("Copying {}", relative_path);
        let action = fs::copy(asset, format!("{}{}{}", root_dir, "/public", relative_path));

        if action.is_err() {
            err_out(format!("Could not copy file {}", relative_path));
        }
    }
}

fn main() {
    const READ_DIR: &str = "../bien.ee";

    // Empty the public dir
    empty_public_dir(READ_DIR);

    // Build individual content items
    compile_content_items(READ_DIR, &compose_global_template_data(READ_DIR));

    // Move assets to /public dir
    copy_assets(READ_DIR);
}
