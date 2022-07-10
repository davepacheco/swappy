//! [`std::fmt::Display`] impl for [`bytesize::ByteSize`]

use bytesize::ByteSize;
use std::fmt::Display;

pub struct ByteSizeDisplayGiB(pub ByteSize);
impl Display for ByteSizeDisplayGiB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_gib = (self.0.as_u64() as f64) / (bytesize::GIB as f64);
        f.write_fmt(format_args!("{:5.1}", as_gib))
    }
}
