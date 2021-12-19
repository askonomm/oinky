use std::fs;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::collections::{HashMap};
use serde::{Serialize, Deserialize};
use comrak::{markdown_to_html, ComrakOptions};
use handlebars::Handlebars;
use regex::Regex;

fn err_out(message: String) {
    print!("{}", message);
    std::process::exit(1);
}

enum FileType {
    Handlebars,
    Markdown,
    Asset,
}

fn find_files(dir: &Path, file_type: FileType) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    if dir.is_dir() {
        let entries = fs::read_dir(dir).unwrap();
        
        for entry in entries {
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
    let partial_file_ends = Vec::from([String::from(".hbs")]);
    let partial_file_paths = find_files(Path::new(&format!("{}{}", root_dir, "/_partials")), &partial_file_ends);
    let mut partials: Vec<TemplatePartial> = Vec::new();

    for path in partial_file_paths {
        let partial_path_split: Vec<&str> = path.split("/").collect();
        let partial_name = partial_path_split.last().copied().unwrap().replace(".hbs", "");

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
    entry: String
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

        let content_item = ContentItem {
            path: file.to_string(),
            slug: file.to_string().replace(root_dir, "").replace(".md", ""),
            meta: meta,
            entry: entry
        };

        content_items.push(content_item);
    }

    return content_items;
}

#[derive(Clone, Serialize, Deserialize)]
struct TemplateData {
    content_item: Option<ContentItem>
}

fn build_html(root_dir: &str, template_name: String, partials: Vec<TemplatePartial>, data: TemplateData) -> String {
    let mut hbs = Handlebars::new();
    
    // Register the main template
    let main_template_path = format!("{}{}{}{}", root_dir, "/_layouts/", template_name, ".hbs");
    let main_template = hbs.register_template_file("_main", &main_template_path);
    
    if main_template.is_err() {
        println!("Something went wrong within your template, {}", template_name);
    }

    // Register partials
    for partial in partials {
        let partial_template = hbs.register_template_file(&partial.name, partial.path);

        if partial_template.is_err() {
            println!("Something went wrong within your partial, {}", partial.name);
        }
    }

    let render = hbs.render("_main", &data);

    if render.is_ok() {
        return render.unwrap();
    }

    return String::new();
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
    };
}

fn write_to_path(path: &str, contents: String) {
    let path = Path::new(&path);
    let prefix = path.parent().unwrap();
    fs::create_dir_all(prefix).unwrap();

    let mut file = File::create(path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file.sync_data().unwrap();
}

fn main() {
    const READ_DIR: &str = "../bien.ee";
    let read_path = Path::new(READ_DIR);
    let markdown_file_ends = Vec::from([String::from(".md")]);
    let content_files = find_files(read_path, &markdown_file_ends);
    let content_items = parse_content_files(READ_DIR, &content_files);
    let partials = find_partials(READ_DIR);

    // Empty public dir
    empty_public_dir(READ_DIR);

    // Build global data
    let data = TemplateData {
        content_item: None,
    };
    
    // Build individual content items
    for content_item in content_items {
        println!("Building {}", content_item.slug);

        let item = content_item.clone();
        let layout = item.meta["layout"].as_str();
        let item_data = TemplateData {
            content_item: Some(content_item),
            ..data
        };

        let html = build_html(READ_DIR, layout.to_string(), partials.clone(), item_data);
        let write_path = format!("{}{}{}{}", READ_DIR, "/public", item.slug, "/index.html");
        write_to_path(&write_path, html);
    }
}