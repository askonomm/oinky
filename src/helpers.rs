use super::{get_config, TemplateData};
use chrono::prelude::*;
use handlebars::{Context, Handlebars, Helper, HelperResult, Output, RenderContext, Renderable};
use regex::Regex;

/// Handlebars date helper.
/// Usage:
///
/// ```handlebars
/// {{date "%Y %d %m"}}
/// ```
pub fn date_helper(
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
pub fn format_date_helper(
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
pub fn is_slug_helper(
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
pub fn unless_slug_helper(
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
