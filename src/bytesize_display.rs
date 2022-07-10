//! [`std::fmt::Display`] impl for [`bytesize::ByteSize`]

use bytesize::ByteSize;
use std::fmt::Display;

/// Formats a [`ByteSize`] for display as a floating-point number of Gibibytes
pub struct ByteSizeDisplayGiB(pub ByteSize);
impl Display for ByteSizeDisplayGiB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_gib = (self.0.as_u64() as f64) / (bytesize::GIB as f64);
        if let Some(width) = f.width() {
            f.write_fmt(format_args!("{:width$.1}", as_gib, width = width))
        } else {
            f.write_fmt(format_args!("{:.1}", as_gib))
        }
    }
}

/// Formats a [`ByteSize`] for display as a number of Kibibytes
pub struct ByteSizeDisplayKiB(pub ByteSize);
impl Display for ByteSizeDisplayKiB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_kib = (self.0.as_u64()) / bytesize::KIB;
        if let Some(width) = f.width() {
            f.write_fmt(format_args!("{:width$.1}", as_kib, width = width))
        } else {
            f.write_fmt(format_args!("{}", as_kib))
        }
    }
}
