//! kstat helper functions and types

use crate::PAGE_SIZE;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use bytesize::ByteSize;

pub fn kstat_read_physmem(
    kstat: &kstat_rs::Ctl,
) -> Result<PhysicalMemoryStats, anyhow::Error> {
    let mut filter = kstat.filter(Some("unix"), Some(0), Some("system_pages"));
    let mut kst =
        filter.next().ok_or_else(|| anyhow!("found no system_pages kstats"))?;
    if filter.next().is_some() {
        bail!("found too many system_pages kstats");
    }

    let data = kstat.read(&mut kst).context("reading kstat")?;
    PhysicalMemoryStats::from_kstat(&data)
}

#[derive(Debug)]
pub struct PhysicalMemoryStats {
    pub freemem: ByteSize,
    physmem: u64,
    availrmem: u64,
    lotsfree: u64,
    desfree: u64,
    minfree: u64,
}

impl PhysicalMemoryStats {
    pub fn from_kstat<'a>(
        kst: &'a kstat_rs::Data<'a>,
    ) -> Result<Self, anyhow::Error> {
        let mut physmem: Option<u64> = None;
        let mut freemem: Option<u64> = None;
        let mut availrmem: Option<u64> = None;
        let mut lotsfree: Option<u64> = None;
        let mut desfree: Option<u64> = None;
        let mut minfree: Option<u64> = None;

        let named = if let kstat_rs::Data::Named(named_stats) = kst {
            named_stats
        } else {
            bail!("expected named kstat for reading physical memory");
        };

        for nst in named {
            let which_value = match nst.name {
                "physmem" => &mut physmem,
                "freemem" => &mut freemem,
                "availrmem" => &mut availrmem,
                "lotsfree" => &mut lotsfree,
                "desfree" => &mut desfree,
                "minfree" => &mut minfree,
                _ => continue,
            };

            if which_value.is_some() {
                bail!("duplicate value for kstat named {:?}", nst.name);
            }

            let value = kstat_value_u64(nst)?;
            *which_value = Some(value);
        }

        Ok(PhysicalMemoryStats {
            physmem: physmem.ok_or_else(|| anyhow!("missing stat physmem"))?,
            freemem: ByteSize::b(
                freemem.ok_or_else(|| anyhow!("missing stat freemem"))?
                    * (PAGE_SIZE as u64),
            ),
            availrmem: availrmem
                .ok_or_else(|| anyhow!("missing stat availrmem"))?,
            lotsfree: lotsfree
                .ok_or_else(|| anyhow!("missing stat lotsfree"))?,
            desfree: desfree.ok_or_else(|| anyhow!("missing stat desfree"))?,
            minfree: minfree.ok_or_else(|| anyhow!("missing stat minfree"))?,
        })
    }
}

fn kstat_value_u64<'a>(
    datum: &'a kstat_rs::Named<'a>,
) -> Result<u64, anyhow::Error> {
    if let kstat_rs::NamedData::UInt64(value) = datum.value {
        Ok(value)
    } else {
        Err(anyhow!(
            "kstat named {:?}: expected u64, found {:?}",
            datum.name,
            datum.value
        ))
    }
}
