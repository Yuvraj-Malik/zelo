//! Instant answers that never need the AI model:
//! - time / date, here or in another city ("time in tokyo")
//! - live weather and rain forecasts from Open-Meteo (free, no key)
use chrono::{Local, Utc};
use chrono_tz::Tz;
use regex::Regex;
use serde_json::Value;
use std::{sync::OnceLock, time::Duration};

pub struct Instant {
    pub title: String,
    pub subtitle: String,
    /// What Enter opens (e.g. a fuller forecast); empty = nothing.
    pub url: String,
}

fn rx(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

fn clean_place(s: &str) -> String {
    s.trim()
        .trim_end_matches('?')
        .trim()
        .trim_start_matches("the ")
        .split(|c| c == ',' || c == '?')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------- time / date

/// A few common city names -> time zones; anything else goes through geocoding.
const CITIES: &[(&str, &str)] = &[
    ("london", "Europe/London"), ("uk", "Europe/London"), ("paris", "Europe/Paris"),
    ("berlin", "Europe/Berlin"), ("dubai", "Asia/Dubai"), ("tokyo", "Asia/Tokyo"),
    ("japan", "Asia/Tokyo"), ("singapore", "Asia/Singapore"), ("sydney", "Australia/Sydney"),
    ("new york", "America/New_York"), ("nyc", "America/New_York"), ("usa", "America/New_York"),
    ("san francisco", "America/Los_Angeles"), ("california", "America/Los_Angeles"),
    ("los angeles", "America/Los_Angeles"), ("seattle", "America/Los_Angeles"),
    ("chicago", "America/Chicago"), ("toronto", "America/Toronto"), ("canada", "America/Toronto"),
    ("india", "Asia/Kolkata"), ("delhi", "Asia/Kolkata"), ("mumbai", "Asia/Kolkata"),
    ("china", "Asia/Shanghai"), ("beijing", "Asia/Shanghai"), ("hong kong", "Asia/Hong_Kong"),
    ("seoul", "Asia/Seoul"), ("korea", "Asia/Seoul"), ("moscow", "Europe/Moscow"),
    ("utc", "UTC"), ("gmt", "UTC"),
];

fn wants_date(t: &str) -> bool {
    (t.contains("date") || t.contains("day")) && !t.contains("time")
}

fn time_query(t: &str) -> Option<Option<String>> {
    static TIME: OnceLock<Regex> = OnceLock::new();
    let r = rx(
        &TIME,
        r"^(?:what(?:'s| is)?\s+(?:the\s+)?)?(?:current\s+)?(?:time|date|day)(?:\s+(?:is it|right now|now|today|is today))?(?:\s+(?:in|at)\s+(.+))?\??$|^(?:what\s+)?time\s+is\s+it(?:\s+(?:in|at)\s+(.+))?\??$|^(?:what(?:'s| is)\s+)?today'?s?\s+date\??$|^what\s+day\s+is\s+(?:it|today)\??$",
    );
    let c = r.captures(t)?;
    let place = c.get(1).or_else(|| c.get(2)).map(|m| clean_place(m.as_str())).filter(|p| !p.is_empty());
    Some(place)
}

fn tz_for(place: &str) -> Option<(Tz, String)> {
    let p = place.to_lowercase();
    if let Some((_, tz)) = CITIES.iter().find(|(c, _)| *c == p) {
        return Some((tz.parse().ok()?, title_case(place)));
    }
    let g = geocode(place)?;
    Some((g.timezone.parse().ok()?, g.label))
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn answer_time(place: Option<String>, date: bool) -> Option<Instant> {
    match place {
        None if date => {
            let now = Local::now();
            Some(Instant {
                title: now.format("%A, %-d %B %Y").to_string(),
                subtitle: format!("{} · week {}", now.format("%-I:%M %p"), now.format("%V")),
                url: String::new(),
            })
        }
        None => {
            let now = Local::now();
            Some(Instant {
                title: now.format("%-I:%M %p").to_string(),
                subtitle: format!("{} · your time", now.format("%A, %-d %B %Y")),
                url: String::new(),
            })
        }
        Some(p) => {
            let (tz, label) = tz_for(&p)?;
            let there = Utc::now().with_timezone(&tz);
            let here = Local::now();
            use chrono::Offset;
            let diff_min = ((there.offset().fix().local_minus_utc() - here.offset().fix().local_minus_utc()) / 60) as i64;
            let diff = if diff_min == 0 {
                "same as you".to_string()
            } else {
                let sign = if diff_min > 0 { "ahead" } else { "behind" };
                let m = diff_min.abs();
                if m % 60 == 0 { format!("{} h {sign}", m / 60) } else { format!("{} h {} min {sign}", m / 60, m % 60) }
            };
            Some(Instant {
                title: format!("{} in {label}", there.format("%-I:%M %p")),
                subtitle: format!("{} · {diff}", there.format("%A, %-d %B")),
                url: String::new(),
            })
        }
    }
}

// ---------------------------------------------------------------- weather

struct Place {
    lat: f64,
    lon: f64,
    label: String,
    timezone: String,
}

fn geocode(name: &str) -> Option<Place> {
    let v: Value = ureq::get("https://geocoding-api.open-meteo.com/v1/search")
        .timeout(Duration::from_secs(4))
        .query("name", name)
        .query("count", "1")
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let r = v["results"].as_array()?.first()?;
    let city = r["name"].as_str().unwrap_or(name);
    let label = match r["admin1"].as_str().or(r["country"].as_str()) {
        Some(region) if region != city => format!("{city}, {region}"),
        _ => city.to_string(),
    };
    Some(Place {
        lat: r["latitude"].as_f64()?,
        lon: r["longitude"].as_f64()?,
        label,
        timezone: r["timezone"].as_str().unwrap_or("UTC").to_string(),
    })
}

/// Your approximate location from your IP, cached for the session.
fn here() -> Option<Place> {
    static CACHE: OnceLock<Option<(f64, f64, String)>> = OnceLock::new();
    let (lat, lon, label) = CACHE
        .get_or_init(|| {
            let v: Value = ureq::get("https://ipwho.is/").timeout(Duration::from_secs(4)).call().ok()?.into_json().ok()?;
            if v["success"].as_bool() != Some(true) {
                return None;
            }
            let city = v["city"].as_str().unwrap_or("your area").to_string();
            Some((v["latitude"].as_f64()?, v["longitude"].as_f64()?, city))
        })
        .clone()?;
    Some(Place { lat, lon, label, timezone: "auto".into() })
}

fn describe(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
        51 | 53 | 55 | 56 | 57 => "Drizzle",
        61 | 63 | 66 => "Rain",
        65 | 67 => "Heavy rain",
        71 | 73 | 75 | 77 => "Snow",
        80 | 81 => "Rain showers",
        82 => "Heavy showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "—",
    }
}

fn icon(code: i64) -> &'static str {
    match code {
        0 | 1 => "☀️",
        2 => "⛅",
        3 => "☁️",
        45 | 48 => "🌫️",
        51..=67 | 80..=82 => "🌧️",
        71..=77 | 85 | 86 => "❄️",
        95..=99 => "⛈️",
        _ => "🌡️",
    }
}

pub struct WeatherQuery {
    place: Option<String>,
    tomorrow: bool,
    rain: bool,
}

fn weather_query(t: &str) -> Option<WeatherQuery> {
    static W: OnceLock<Regex> = OnceLock::new();
    let r = rx(&W, r"\b(weather|temperature|temp|forecast|rain|raining|umbrella|hot|cold|humid|humidity)\b");
    if !r.is_match(t) {
        return None;
    }
    // "how hot is the sun" shouldn't be weather; require a weather noun or a rain/umbrella word.
    static NOUN: OnceLock<Regex> = OnceLock::new();
    if !rx(&NOUN, r"\b(weather|temperature|temp|forecast|rain|raining|umbrella|humidity)\b").is_match(t)
        && !t.contains("outside")
    {
        return None;
    }
    static IN: OnceLock<Regex> = OnceLock::new();
    let place = rx(&IN, r"\b(?:in|at|for)\s+([a-z][a-z .'-]+?)(?:\s+(?:today|tomorrow|now|right now|this week))?\??$")
        .captures(t)
        .map(|c| clean_place(&c[1]))
        .filter(|p| !p.is_empty() && !matches!(p.as_str(), "today" | "tomorrow" | "now"));
    Some(WeatherQuery { place, tomorrow: t.contains("tomorrow"), rain: t.contains("rain") || t.contains("umbrella") })
}

fn answer_weather(w: WeatherQuery) -> Option<Instant> {
    let place = match &w.place {
        Some(p) => geocode(p)?,
        None => here()?,
    };
    let v = fetch_forecast(&place)?;
    format_weather(&w, &place.label, &v)
}

fn fetch_forecast(place: &Place) -> Option<Value> {
    ureq::get("https://api.open-meteo.com/v1/forecast")
        .timeout(Duration::from_secs(5))
        .query("latitude", &place.lat.to_string())
        .query("longitude", &place.lon.to_string())
        .query("current", "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m")
        .query("daily", "temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code")
        .query("timezone", if place.timezone.is_empty() { "auto" } else { &place.timezone })
        .query("forecast_days", "2")
        .call()
        .ok()?
        .into_json()
        .ok()
}

fn format_weather(w: &WeatherQuery, label: &str, v: &Value) -> Option<Instant> {
    let place_label = label.to_string();
    let day = if w.tomorrow { 1 } else { 0 };
    let d = &v["daily"];
    let hi = d["temperature_2m_max"][day].as_f64()?.round();
    let lo = d["temperature_2m_min"][day].as_f64()?.round();
    let rain = d["precipitation_probability_max"][day].as_f64().unwrap_or(0.0).round();
    let url = format!("https://www.google.com/search?q={}", urlencoding_encode(&format!("weather {place_label}")));

    if w.tomorrow {
        let code = d["weather_code"][1].as_i64().unwrap_or(-1);
        let lead = if w.rain {
            if rain >= 50.0 { format!("Yes, likely rain tomorrow ({rain}%)") } else { format!("Rain unlikely tomorrow ({rain}%)") }
        } else {
            format!("{} {}, {hi}° / {lo}°", icon(code), describe(code))
        };
        return Some(Instant {
            title: format!("{lead} in {place_label}"),
            subtitle: format!("Tomorrow · High {hi}° · Low {lo}° · Rain chance {rain}%"),
            url,
        });
    }

    let c = &v["current"];
    let temp = c["temperature_2m"].as_f64()?.round();
    let feels = c["apparent_temperature"].as_f64().unwrap_or(temp).round();
    let code = c["weather_code"].as_i64().unwrap_or(-1);
    let hum = c["relative_humidity_2m"].as_f64().unwrap_or(0.0).round();
    let wind = c["wind_speed_10m"].as_f64().unwrap_or(0.0).round();
    let title = if w.rain {
        let verdict = if (51..=67).contains(&code) || (80..=99).contains(&code) {
            "It's raining now".to_string()
        } else if rain >= 50.0 {
            format!("Rain likely later today ({rain}%)")
        } else {
            format!("No rain expected today ({rain}%)")
        };
        format!("{verdict} in {place_label}")
    } else {
        format!("{} {temp}°C, {} in {place_label}", icon(code), describe(code))
    };
    Some(Instant {
        title,
        subtitle: format!("Feels like {feels}° · High {hi}° · Low {lo}° · Humidity {hum}% · Wind {wind} km/h · Rain {rain}%"),
        url,
    })
}

fn urlencoding_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Cheap check the UI can use before any network call.
pub fn matches(q: &str) -> bool {
    let t = q.trim().to_lowercase();
    time_query(&t).is_some() || weather_query(&t).is_some()
}

/// Full answer (may hit the network for weather or a city's time zone).
pub fn answer(q: &str) -> Option<Instant> {
    let t = q.trim().to_lowercase();
    if let Some(place) = time_query(&t) {
        return answer_time(place, wants_date(&t));
    }
    if let Some(w) = weather_query(&t) {
        return answer_weather(w);
    }
    None
}
