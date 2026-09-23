mod base;
mod generic;
mod douyin;
mod bilibili;
mod kuaishou;
mod huya;
mod douyu;
mod yy;

pub use base::{PlatformHandler, PlatformRegistry, StreamData};
pub use douyin::DouyinHandler;
pub use bilibili::BilibiliHandler;
pub use generic::GenericHandler;
pub use kuaishou::KuaishouHandler;
pub use huya::HuyaHandler;
pub use douyu::DouyuHandler;
pub use yy::YYHandler;
