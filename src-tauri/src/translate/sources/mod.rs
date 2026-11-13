//! 翻译源集合：trait 能力声明 + 静态注册表 + 各源实现。
//! 加一个翻译源 = 新建一个文件 + 在下面 SOURCES 登记一行（分发、降级链、对比区自动跟上）。

use std::sync::OnceLock;
use std::time::Duration;

use crate::translate::{AppConfig, WordInfo};

/// 网页端免 Key 接口都按浏览器 UA 放行
pub(crate) const WEB_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// 降级源单独给短超时：境外端点不通时会挂到 15s，把整条链拖死
pub(crate) const FALLBACK_TIMEOUT: Duration = Duration::from_secs(6);

/// 全局共享的 HTTP 客户端：连接池只建一次（曾每次 Client::new()，高频调用内存缓慢上涨）
pub(crate) fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        // 整体超时：服务端无响应时不能让弹窗永远停在“翻译中...”
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client")
    })
}

/// trait 方法返回的 boxed future：async fn 进 trait 会带来无法对象化的 opaque future，
/// 手动 Box 保持注册表用 trait 对象（&dyn）遍历分发
pub(crate) type TranslateFut<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<WordInfo, String>> + Send + 'a>>;

/// 翻译源能力接口。cfg 按引用传入（API key 不塞构造器，避免配置生命周期问题）
pub(crate) trait TranslateSource: Sync {
    /// 能力声明：是否支持「from → to」（语言代码 "zh" | "en"）。
    /// pipeline 据此统一过滤，方向限制只在源内声明一处（如有道 jsonapi 仅英→中）
    fn supports(&self, from: &str, to: &str) -> bool {
        let _ = (from, to);
        true
    }

    /// 前置条件（密钥等）是否就绪：未就绪的源在逐源尝试时被跳过（自动降级到下一个源）
    fn ready(&self, cfg: &AppConfig) -> bool {
        let _ = cfg;
        true
    }

    /// 执行翻译。返回 boxed future 以保持 trait 对象安全
    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        cfg: &'a AppConfig,
    ) -> TranslateFut<'a>;
}

/// 静态注册表：编译期已知，数组遍历即实现分发与降级。
/// ID 放元组而非 trait 关联常量：关联常量无法经 trait 对象访问
static SOURCES: &[(&str, &dyn TranslateSource)] = &[
    ("youdao", &youdao_dict::YoudaoDict),
    ("bing", &bing::Bing),
    ("youdaoai", &youdao_ai::YouDaoAi),
    ("glm", &glm::Glm),
    ("deepl", &deepl::Deepl),
];

/// 按 id 取源实现；未知 id（老配置残留等）返回 None
pub(crate) fn find_source(id: &str) -> Option<&'static dyn TranslateSource> {
    SOURCES.iter().find(|(s, _)| *s == id).map(|(_, v)| *v)
}

pub(crate) mod bing;
pub(crate) mod deepl;
pub(crate) mod glm;
pub(crate) mod youdao_ai;
pub(crate) mod youdao_dict;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_declared_sources() {
        for id in ["youdao", "bing", "youdaoai", "glm", "deepl"] {
            assert!(find_source(id).is_some(), "注册表缺源: {id}");
        }
        assert!(find_source("nope").is_none());
    }
}
