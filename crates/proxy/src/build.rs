pub use crate::channel::target::exec::ExecBuilder;
pub use crate::channel::target::stream::StreamBuilder;
use crate::channel::target::{Exec, Stream};
pub use crate::format::SqlFormat;

pub fn sql_format() -> SqlFormat {
    SqlFormat::builder()
}

pub fn stream() -> StreamBuilder {
    Stream::builder()
}

pub fn exec() -> ExecBuilder {
    Exec::builder()
}
