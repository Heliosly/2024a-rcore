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













/// 将一个绝对路径字符串规范化。
///
/// 规范化包括：
/// - 解析 "." 和 ".." 路径组件。
/// - 移除重复的斜杠 (例如 "/a//b" -> "/a/b")。
/// - 确保结果以 "/" 开头（除非原始路径是空且被视为错误）。
/// - 不解析符号链接。
///
/// # 参数
/// * `abs_path`: 一个表示绝对路径的字符串切片。必须以 "/" 开头。
///
/// # 返回
/// 规范化后的绝对路径 `String`。
/// 如果输入路径无效（例如，".." 尝试上溯到根目录之上），行为可能取决于具体实现，
/// 但这里我们会尽量保持在根目录下。
///
/// # Panics
/// 如果 `abs_path` 不是以 "/" 开头
pub fn normalize_absolute_path(abs_path: &str) -> String {
    if !abs_path.starts_with('/') {
        panic!("normalize_absolute_path: Input path '{}' is not absolute.", abs_path);
    }
    if abs_path.is_empty() {
        panic!("normalize_absolute_path: Input path cannot be empty.");
    }

    let mut components: Vec<&str> = Vec::new();

    for component in abs_path.split('/') {
        match component {
            "" | "." => {
                // 忽略空组件 (来自 "///" 或结尾的 "/") 和 "." 组件
                // 如果是路径的第一个组件且为空 (来自开头的 "/")，是正常的，
            }
            ".." => {
                // 处理 ".." 组件，从 components 中弹出一个
                components.pop();
            }
            _ => {
                // 其他有效组件，加入 components
                components.push(component);
            }
        }
    }

    if components.is_empty() {
        // 如果所有组件都被处理掉 (例如 "/../.." 或 "/.")，结果是根目录
        "/".to_string()
    } else {
        // 以 "/" 开头，然后用 "/" 连接所有组件
        format!("/{}", components.join("/"))
    }
}


/// 从一个已规范化的绝对路径中分离出父目录路径和最后一个组件。
///
/// # 参数
/// * `normalized_abs_path`: 一个已规范化的绝对路径字符串。
///   它保证以 "/" 开头，不包含 "." 或 ".." 组件，且没有重复的 "/"。
///
/// # 返回
/// `(String, String)`: (父目录路径, 文件名/最后一个组件名)
/// * 对于 `"/foo/bar"`，返回 `("/foo", "bar")`
/// * 对于 `"/foo"`，返回 `("/", "foo")`
/// * 对于 `"/"`，返回 `("/", "/")` (父是根，名也是根，特殊处理)
///
/// # Panics
/// 如果 `normalized_abs_path` 不是以 "/" 开头或为空，这违反了前提。
pub fn get_parent_path_and_filename(normalized_abs_path: &str) -> (String, String) {
    if !normalized_abs_path.starts_with('/') || normalized_abs_path.is_empty() {
        panic!(
            "get_parent_path_and_filename: Input path '{}' is not a normalized absolute path.",
            normalized_abs_path
        );
    }

    if normalized_abs_path == "/" {
        // 根目录的特殊情况
        return ("/".to_string(), "/".to_string());
    }

    // normalized_abs_path 保证不以 '/' 结尾 (除非是根目录本身)
    // 这是 normalize_absolute_path 的一个副作用 (components.join("/") 不会加结尾的'/')
    // 但如果原始路径是 "/a/c/"，normalize_absolute_path 会返回 "/a/c"
    // 所以，我们不需要 strip_suffix('/')。

    // 查找最后一个 '/'
    // 由于路径已规范化且不是 "/"，它必定包含至少一个 '/' 在开头，
    // 并且如果它有多于一个组件，例如 "/foo/bar"，rfind('/') 会找到分隔符。
    // 如果它是 "/foo"，rfind('/') 会找到开头的 '/'。
    match normalized_abs_path.rfind('/') {
        Some(0) => {
            // 路径形式为 "/component" (例如 "/foo")
            // 最后一个 '/' 是在索引 0
            // 父路径是 "/"
            // 文件名是 component (从索引 1 开始)
            ( "/".to_string(), normalized_abs_path[1..].to_string() )
        }
        Some(idx) => {
            // 路径形式为 "/path/to/component" (例如 "/foo/bar")
            // 父路径是索引 idx 之前的部分 (包括开头的 '/')
            // 文件名是索引 idx + 1 之后的部分
            ( normalized_abs_path[0..idx].to_string(), normalized_abs_path[idx+1..].to_string() )
        }
        None => {
            // 对于一个以 "/" 开头的非空、非 "/" 的规范化绝对路径，
            // rfind('/') 必定是 Some(0) 或 Some(idx > 0)。
            // 所以这个 None 分支理论上不应该被触发。
            unreachable!(
                "Normalized absolute path '{}' should contain '/' if not root.",
                normalized_abs_path
            );
        }
    }
}