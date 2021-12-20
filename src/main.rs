use comrak::{markdown_to_html, ComrakOptions};
use handlebars::Handlebars;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

/// Prints an error `message` to shell and subsequently
/// exits the program.
fn err_out(message: String) {
    print!("{}", message);
    std::process::exit(1);
}

enum FileType {
    Handlebars,
    Markdown,
    Asset,
}

/// adsasdasd
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
                    if path_str.ends_with(".hbs") {
                        files.push(path_str);
                    }
                }
                FileType::Markdown => {
                    if path_str.ends_with(".md") {
                        files.push(path_str);
                    }
                }
                FileType::Asset => {
                    if path_str.ends_with(".css") || path_str.ends_with(".js") {
                        files.push(path_str);
                    }
                }
            }
        }
    }

    return files;
}

#[derive(Clone)]
struct TemplatePartial {
    name: String,
    path: String,
}

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

#[derive(Clone, Serialize, Deserialize)]
struct ContentItem {
    path: String,
    slug: String,
    meta: HashMap<String, String>,
    entry: String,
    time_to_read: usize,
}

fn parse_content_file_meta(contents: &str) -> HashMap<String, String> {
    let regex = Regex::new(r"(?s)^(---)(.*?)(---|\.\.\.)").unwrap();
    let meta_block = regex.find(&contents).unwrap().as_str();
    let meta_lines = meta_block.lines();
    let mut map: HashMap<String, String> = HashMap::new();

    for line in meta_lines {
        if line != "---" {
            let split_line: Vec<&str> = line.split(":").collect();
            let key = split_line[0].trim().to_string();
            let val = split_line[1].trim().to_string();

            map.insert(key, val);
        }
    }

    return map;
}

fn parse_content_file_entry(contents: &str) -> String {
    let regex = Regex::new(r"(?s)^---(.*?)---*").unwrap();
    let entry = regex.replace(&contents, "");

    return markdown_to_html(&entry, &ComrakOptions::default());
}

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

#[derive(Clone, Serialize, Deserialize)]
struct TemplateData {
    site: SiteInfo,
    current: Option<ContentItem>,
    content: HashMap<String, Vec<ContentItem>>,
}

fn build_html(
    root_dir: &str,
    template_name: String,
    partials: Vec<TemplatePartial>,
    data: TemplateData,
) -> String {
    let mut hbs = Handlebars::new();

    // Register the main template
    let main_template_path = format!("{}{}{}{}", root_dir, "/_layouts/", template_name, ".hbs");
    let main_template = hbs.register_template_file("_main", &main_template_path);

    if main_template.is_err() {
        println!(
            "Something went wrong within your template, {}: {:?}",
            template_name,
            main_template.err()
        );
    }

    // Register partials
    for partial in partials {
        let partial_template = hbs.register_template_file(&partial.name, partial.path);

        if partial_template.is_err() {
            println!(
                "Something went wrong within your partial, {}: {:?}",
                partial.name,
                partial_template.err()
            );
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

fn write_to_path(path: &str, contents: String) {
    let path = Path::new(&path);
    let prefix = path.parent().unwrap();
    fs::create_dir_all(prefix).unwrap();

    let mut file = File::create(path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file.sync_data().unwrap();
}

fn build_content_items(root_dir: &str, data: &TemplateData) {
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

        let html = build_html(
            root_dir,
            item.meta["layout"].as_str().to_string(),
            partials.clone(),
            item_data,
        );
        let write_path = format!("{}{}{}{}", root_dir, "/public", item.slug, "/index.html");
        write_to_path(&write_path, html);
    }
}

fn build_content_from_dsl(root_dir: &str) -> HashMap<String, Vec<ContentItem>> {
    return HashMap::new();
}

#[derive(Clone, Serialize, Deserialize)]
struct SiteInfo {
    title: Option<String>,
    url: Option<String>,
}

fn get_site_info(root_dir: &str) -> SiteInfo {
    let file_contents = fs::read_to_string(format!("{}{}", root_dir, "/site.json"));
    let contents = file_contents.unwrap_or_default();
    let data = serde_json::from_str(&contents);

    if data.is_err() {
        err_out("Could not read site info from site.json.".to_string());

        return SiteInfo {
            title: None,
            url: None,
        };
    }

    return data.unwrap();
}

fn move_assets(root_dir: &str) {
    let assets = find_files(Path::new(root_dir), &FileType::Asset);
}

fn main() {
    const READ_DIR: &str = "../bien.ee";
    let site_info = get_site_info(READ_DIR);
    let content = build_content_from_dsl(READ_DIR);

    // Empty the public dir
    empty_public_dir(READ_DIR);

    // Build global data
    let data = TemplateData {
        site: site_info,
        current: None,
        content: HashMap::new(),
    };

    // Build individual content items
    build_content_items(READ_DIR, &data);

    // Move assets to /public dir
    move_assets(READ_DIR);
}
