use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

// pub fn trim_first_point_slash(path: &str) -> &str {
//     if path.starts_with("./") {
//         &path[2..]
//     } else {
//         &path
//     }
// }
#[inline(always)]
pub fn trim_start_slash(s: String) -> String {
    if s.chars().take_while(|c| *c == '/').count() >= 2 {
        format!("/{}", s.trim_start_matches('/'))
    } else {
        s
    }
}

pub fn path2abs<'a>(cwdv: &mut Vec<&'a str>, pathv: &Vec<&'a str>) -> String {
    for &path_element in pathv.iter() {
        if path_element == "." {
            continue;
        } else if path_element == ".." {
            cwdv.pop();
        } else {
            cwdv.push(path_element);
        }
    }
    let mut abs_path = String::from("/");
    abs_path.push_str(&cwdv.join("/"));
    abs_path
}

#[inline(always)]
pub fn path2vec(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

#[inline(always)]
pub fn is_abs_path(path: &str) -> bool {
    path.starts_with("/")
}
/// 用于路径拆分
pub fn rsplit_once<'a>(s: &'a str, delimiter: &str) -> (&'a str, &'a str) {
    let (mut parent_path, child_name) = s.rsplit_once(delimiter).unwrap();
    if parent_path.is_empty() {
        parent_path = "/";
    }
    (parent_path, child_name)
}

pub fn get_abs_path(base_path: &str, path: &str) -> String {
    if is_abs_path(&path) {
        path.to_string()
    } else {
        let mut wpath = {
            if base_path == "/" {
                Vec::with_capacity(32)
            } else {
                path2vec(base_path)
            }
        };
        path2abs(&mut wpath, &path2vec(&path))
    }
}

// pub fn strip_color(s: String, prefix: &str, suffix: &str) -> String {
//     debug!("prefix is {}, suffix is {}", prefix, suffix);
//     let trimmed_start = s.strip_prefix(prefix).unwrap_or(&s);
//     let trimmed_result = trimmed_start.strip_suffix(suffix).unwrap_or(trimmed_start);
//     let ret = String::from("ltp/testcases/bin/") + trimmed_result;
//     ret
// }

// use regex::Regex;

// pub fn remove_ansi_escape_sequences(text: &str) -> String {
//     // 定义匹配 ANSI 转义序列的正则表达式
//     let ansi_escape = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
//     // 替换所有匹配的 ANSI 转义序列为空字符串
//     ansi_escape.replace_all(text, "").to_string()
// }

// pub fn strip_color(s: String, prefix: &str, suffix: &str) -> String {
//     let mut tmp = s.replace(prefix, "");
//     tmp = tmp.replace(suffix, "");
//     log::info!("after_strip s={}", &tmp);
//     tmp
// }
