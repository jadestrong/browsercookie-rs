use ini::Ini;
use regex::Regex;
use std::fs::copy;
use std::error::Error;
use cookie::{Cookie, CookieJar};
use std::path::{Path, PathBuf};
use rusqlite::{params, Connection};
use tempdir::TempDir;
use dirs::home_dir;

use crate::errors::BrowsercookieError;

#[allow(non_snake_case)]
#[derive(Debug)]
struct MozCookie {
    host: String,
    name: String,
    path: String,
    value: String,
    secure: bool,
    httponly: bool
}

fn get_master_profile_path() -> PathBuf {
    let mut path = home_dir().expect("Unable to find home directory");
    if cfg!(target_os = "macos") {
        path.push("Library/Application Support/Firefox/profiles.ini");
    } else if cfg!(target_os = "linux") {
        path.push(".mozilla/firefox/profiles.ini")
    }
    path
}

fn get_default_profile_path(master_profile: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let profiles_conf: Ini;
    let mut default_profile_path = PathBuf::from(master_profile);
    default_profile_path.pop();

    match Ini::load_from_file(&master_profile) {
        Err(_) => return Err(Box::new(BrowsercookieError::InvalidProfile(String::from("Unable to parse firefox ini profile")))),
        Ok(p) => profiles_conf = p
    }

    for (_, section) in &profiles_conf {
        match section.get("Path") {
            Some(path) => {
                default_profile_path.push(path);
                break
            }
            None => {}
        }
    }
    Ok(default_profile_path)
}

// fn load_from_recovery(recovery_path: &Path, bcj: &mut Box<CookieJar>, domain_regex: &Regex) -> Result<bool, Box<dyn Error>> {
//     let recovery_file = File::open(recovery_path)?;
//     let recovery_mmap = unsafe { MmapOptions::new().map(&recovery_file)? };

//     if recovery_mmap.len() <= 8 || recovery_mmap.get(0..8).ok_or("Invalid recovery")? != "mozLz40\0".as_bytes() {
//         return Err(Box::new(BrowsercookieError::InvalidRecovery(String::from("Firefox invalid recovery archive"))))
//     }

//     let mut rdr = Cursor::new(recovery_mmap.get(8..12).ok_or("Invalid recovery")?);
//     let uncompressed_size = rdr.read_i32::<LittleEndian>().ok();

//     let recovery_json_bytes = decompress(recovery_mmap.get(12..).ok_or("Invalid recovery")?, uncompressed_size)?;

//     let recovery_json: Value = serde_json::from_slice(&recovery_json_bytes)?;
//     for c in recovery_json["cookies"].as_array().ok_or("Invalid recovery")? {
//         if let Ok(cookie) = serde_json::from_value(c.clone()) as Result<MozCookie, serde_json::error::Error> {
//             // println!("Loading for {}: {}={}", cookie.host, cookie.name, cookie.value);
//             if domain_regex.is_match(&cookie.host) {
//                 bcj.add(Cookie::build(cookie.name, cookie.value)
//                                  .domain(cookie.host)
//                                  .path(cookie.path)
//                                  .secure(cookie.secure)
//                                  .http_only(cookie.httponly)
//                                  .finish());
//             }
//         }
//     }
//     Ok(true)
// }

fn load_from_sqlite(cookie_path: &Path, bcj: &mut Box<CookieJar>, domain_regex: &Regex) -> Result<bool, Box<dyn Error>> {
    let tmp_dir = TempDir::new("ff_cookies")?;
    let cookie_tmp = tmp_dir.path().join("cookie.sqlite");
    copy(cookie_path, &cookie_tmp)?;
    if let Ok(cookies_db) = Connection::open(cookie_tmp) {
        let mut stmt = cookies_db.prepare("select * from moz_cookies")?;
        let cookie_iter = stmt.query_map(params![], |row| {
            Ok(MozCookie {
                name: row.get(2)?,
                value: row.get(3)?,
                host: row.get(4)?,
                path: row.get(5)?,
                secure: row.get(8)?,
                httponly: row.get(9)?,
            })
        })?;
        for cookie in cookie_iter {
            let cookie = cookie?;
            if domain_regex.is_match(&cookie.host) {
                bcj.add(Cookie::build(cookie.name, cookie.value).domain(cookie.host).finish());
            }
        }
        Ok(true)
    } else {
        Err(Box::new(BrowsercookieError::InvalidCookieStore(String::from("Firefox invalid cookie store"))))
    }
}

pub(crate) fn load(bcj: &mut Box<CookieJar>, domain_regex: &Regex) -> Result<(), Box<dyn Error>>  {
    // Returns a CookieJar on heap if following steps go right
    //
    // 1. Get default profile path for firefox from master ini profiles config.
    // 2. Load cookies from recovery json (sessionstore-backups/recovery.jsonlz4)
    //    of the default profile.
    let master_profile_path = get_master_profile_path();
    if !master_profile_path.exists() {
        return Err(Box::new(BrowsercookieError::ProfileMissing(String::from("Firefox profile path doesn't exist"))))
    }

    let mut profile_path = get_default_profile_path(&master_profile_path)?;
    profile_path.push("cookies.sqlite");
    if !profile_path.exists() {
        return Err(Box::new(BrowsercookieError::InvalidCookieStore(String::from("Firefox invalid cookie store"))))
    }
    load_from_sqlite(&profile_path, bcj, domain_regex)?;

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_load(){
        let mut bcj = Box::new(CookieJar::new());

        let domain_re = Regex::new(".*").unwrap();
        load(&mut bcj, &domain_re).expect("Failed to load from firefox");
    }

    // #[test]
    // fn test_recovery_load() {
    //     let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    //     path.push("tests/resources/recovery.jsonlz4");
    //     println!("path: {:?}", path);
    //     let mut bcj = Box::new(CookieJar::new());

    //     let domain_re = Regex::new(".*").unwrap();
    //     load_from_recovery(&path, &mut bcj, &domain_re).expect("Failed to load from firefox recovery json");

    //     let c = bcj.get("taarId").expect("Failed to get cookie from firefox recovery");

    //     assert_eq!(c.value(), "value");
    //     assert_eq!(c.path(), Some("/"));
    //     assert_eq!(c.secure(), Some(true));
    //     assert_eq!(c.http_only(), Some(true));
    //     assert_eq!(c.domain(), Some("addons.mozilla.org"));
    // }

    #[test]
    fn test_master_profile() {
        // let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // path.push("tests/resources/profiles.ini");

        let path = get_master_profile_path();
        let default_profile_path = get_default_profile_path(&path).expect("Failed to parse master firefox profile");

        assert!(default_profile_path.exists());
        // assert!(default_profile_path.ends_with(PathBuf::from("Profiles/1qbuu7ux.default")));
    }
}
