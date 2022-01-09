mod dsl;
mod helpers;
mod utils;

use cached::proc_macro::cached;
use comrak::{markdown_to_html, ComrakOptions};
use dotenv::dotenv;
use dsl::{TemplateContentDSLItem};
use handlebars::Handlebars;
use hotwatch::{Event, Hotwatch};
use parking_lot;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json;
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
pub struct ContentItem {
    path: String,
    slug: String,
    meta: HashMap<String, String>,
    entry: String,
    time_to_read: usize,
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
    return Config {
        dir: env::var("READ_DIR")
            .unwrap_or(env::current_dir().unwrap().to_str().unwrap().to_string())
            .to_string(),
        utc_offset: env::var("UTC_OFFSET")
            .unwrap_or(0.to_string())
            .parse::<i32>()
            .unwrap(),
    };
}

/// Determines if the given `path` matches a Handlebars file.
fn is_handlebars_file(path: &str) -> bool {
    let relative_path = path.replace(&get_config().dir, "");

    return !relative_path.starts_with("/public")
        && !relative_path.starts_with("/node_modules")
        && (path.ends_with(".hbs") || path.ends_with(".handlebars"));
}

/// Determines if the given `path` matches a Handlebars Page file.
fn is_handlebars_page_file(path: &str) -> bool {
    let relative_path = path.replace(&get_config().dir, "");

    return !relative_path.starts_with("/_layouts")
        && !relative_path.starts_with("/_partials")
        && !relative_path.starts_with("/public")
        && !relative_path.starts_with("/node_modules")
        && (path.ends_with(".hbs") || path.ends_with(".handlebars"));
}

/// Determines if the given `path` matches a Markdown file.
fn is_markdown_file(path: &str) -> bool {
    let relative_path = path.replace(&get_config().dir, "");

    return !relative_path.starts_with("/_layouts")
        && !relative_path.starts_with("/_partials")
        && !relative_path.starts_with("/public")
        && !relative_path.starts_with("/node_modules")
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
        && relative_path != "/site.json"
        && relative_path != "/content.json"
        && !relative_path.starts_with("/_layouts")
        && !relative_path.starts_with("/_partials")
        && !relative_path.starts_with("/public")
        && !relative_path.starts_with("/node_modules")
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
    let mut opts = ComrakOptions::default();
    opts.render.unsafe_ = true;

    return markdown_to_html(&entry, &opts);
}

/// Parses given Markdown `files` for contents that contain YAML-like meta-data
/// and the Markdown entry. Returns a vector of `ContentItem`.
#[cached(time = 2)]
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
    hbs.register_helper("date", Box::new(helpers::date_helper));
    hbs.register_helper("format_date", Box::new(helpers::format_date_helper));
    hbs.register_helper("is_slug", Box::new(helpers::is_slug_helper));
    hbs.register_helper("unless_slug", Box::new(helpers::unless_slug_helper));

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
}

/// Compiles all content items within the root directory with given
/// global Handlebars `data`, resulting in HTML files written to disk.
fn compile_content_items(data: TemplateData) {
    let content_files = find_files(get_config().dir, FileType::Markdown);
    let content_items = parse_content_files(content_files);
    let chunks = content_items.chunks(50).map(|c| c.to_owned());
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
    let chunks = template_files.chunks(50).map(|c| c.to_owned());
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

/// Composes global template data for consumption by Handlebars templates.
#[cached(time = 2)]
fn compose_global_template_data() -> TemplateData {
    return TemplateData {
        site: get_site_info(),
        content: dsl::compose_content_from_dsl(),
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
