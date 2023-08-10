// Copyright (c) Facebook, Inc. and its affiliates.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::str::FromStr;

use anyhow::bail;
use anyhow::Error;
use anyhow::Result;
use clap::Parser;
use model::BtrfsModelFieldId;
use model::FieldId;
use model::NetworkModelFieldId;
use model::SingleCgroupModelFieldId;
use model::SingleDiskModelFieldId;
use model::SingleNetModelFieldId;
use model::SingleProcessModelFieldId;
use model::SystemModelFieldId;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::CommonField;
use crate::DumpField;

/// Field that represents a group of related FieldIds of a Queriable.
/// Shorthand for specifying fields to dump.
pub trait AggField<F: FieldId> {
    fn expand(&self, detail: bool) -> Vec<F>;
}

/// Generic representation of fields accepted by different dump subcommands.
/// Each DumpOptionField is either an aggregation of multiple FieldIds, or a
/// "unit" field which could be either a CommonField or a FieldId.
#[derive(Clone, Debug, PartialEq)]
pub enum DumpOptionField<F: FieldId, A: AggField<F>> {
    Unit(DumpField<F>),
    Agg(A),
}

/// Expand the Agg fields and collect them with other Unit fields.
pub fn expand_fields<F: FieldId + Clone, A: AggField<F>>(
    fields: &[DumpOptionField<F, A>],
    detail: bool,
) -> Vec<DumpField<F>> {
    let mut res = Vec::new();
    for field in fields {
        match field {
            DumpOptionField::Unit(field) => res.push(field.clone()),
            DumpOptionField::Agg(agg) => {
                res.extend(agg.expand(detail).into_iter().map(DumpField::FieldId))
            }
        }
    }
    res
}

/// Used by Clap to parse user provided --fields.
impl<F: FieldId + FromStr, A: AggField<F> + FromStr> FromStr for DumpOptionField<F, A> {
    type Err = Error;

    /// When parsing command line options into DumpOptionField, priority order
    /// is CommonField, AggField, and then FieldId.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(common) = CommonField::from_str(s) {
            Ok(Self::Unit(DumpField::Common(common)))
        } else if let Ok(agg) = A::from_str(s) {
            Ok(Self::Agg(agg))
        } else if let Ok(field_id) = F::from_str(s) {
            Ok(Self::Unit(DumpField::FieldId(field_id)))
        } else {
            bail!("Variant not found: {}", s);
        }
    }
}

/// Used for generating help string that lists all supported fields.
impl<F: FieldId + ToString, A: AggField<F> + ToString> ToString for DumpOptionField<F, A> {
    fn to_string(&self) -> String {
        match self {
            Self::Unit(DumpField::Common(common)) => common.to_string(),
            Self::Unit(DumpField::FieldId(field_id)) => field_id.to_string(),
            Self::Agg(agg) => agg.to_string(),
        }
    }
}

/// Join stringified items with ", ". Used for generating help string that lists
/// all supported fields.
fn join(iter: impl IntoIterator<Item = impl ToString>) -> String {
    iter.into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

// make_option macro will build a enum of tags that map to string values by
// implementing the FromStr trait.
// This is useful when are trying to processing or display fields base on
// a user's input. Here's a use case:
// We display fields in the order of user's input. After we got
// the input array, dfill trait will automatically generate a vec of fns base
// on that array. For example, user input `--fields cpu_usage cpu_user`,
// enum generated by make_option will auto translate string to enum tags. After
// that dfill trait will generate `vec![print_cpu_usage, print_cpu_user]`. And
// the dprint trait will just iterate over the fns and call it with current model.
//
// Another user case is for the select feature, we don't want a giant match
// of string patterns once user select some field to do some operations. Instead,
// we can use a match of enum tags, that will be much faster.
macro_rules! make_option {
    ($name:ident {$($str_field:tt: $enum_field:ident,)*}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Hash, Eq)]
        pub enum $name {
            $($enum_field,)*
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(opt: &str) -> Result<Self> {
                match opt.to_lowercase().as_str() {
                    $($str_field => Ok($name::$enum_field),)*
                    _ => bail!("Fail to parse {}", opt)
                }
            }
        }
    }
}

/// Represents the four sub-model of SystemModel.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum SystemAggField {
    Cpu,
    Mem,
    Vm,
    Stat,
}

impl AggField<SystemModelFieldId> for SystemAggField {
    fn expand(&self, detail: bool) -> Vec<SystemModelFieldId> {
        use model::MemoryModelFieldId as Mem;
        use model::ProcStatModelFieldId as Stat;
        use model::SingleCpuModelFieldId as Cpu;
        use model::SystemModelFieldId as FieldId;
        use model::VmModelFieldId as Vm;

        if detail {
            match self {
                Self::Cpu => enum_iterator::all::<Cpu>()
                    // The Idx field is always -1 (we aggregate all CPUs)
                    .filter(|v| v != &Cpu::Idx)
                    .map(FieldId::Cpu)
                    .collect(),
                Self::Mem => enum_iterator::all::<Mem>().map(FieldId::Mem).collect(),
                Self::Vm => enum_iterator::all::<Vm>().map(FieldId::Vm).collect(),
                Self::Stat => enum_iterator::all::<Stat>().map(FieldId::Stat).collect(),
            }
        } else {
            // Default fields for each group
            match self {
                Self::Cpu => vec![Cpu::UsagePct, Cpu::UserPct, Cpu::SystemPct]
                    .into_iter()
                    .map(FieldId::Cpu)
                    .collect(),
                Self::Mem => vec![Mem::Total, Mem::Free]
                    .into_iter()
                    .map(FieldId::Mem)
                    .collect(),
                Self::Vm => enum_iterator::all::<Vm>().map(FieldId::Vm).collect(),
                Self::Stat => enum_iterator::all::<Stat>().map(FieldId::Stat).collect(),
            }
        }
    }
}

pub type SystemOptionField = DumpOptionField<SystemModelFieldId, SystemAggField>;

pub static DEFAULT_SYSTEM_FIELDS: &[SystemOptionField] = &[
    DumpOptionField::Unit(DumpField::FieldId(SystemModelFieldId::Hostname)),
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Agg(SystemAggField::Cpu),
    DumpOptionField::Agg(SystemAggField::Mem),
    DumpOptionField::Agg(SystemAggField::Vm),
    DumpOptionField::Unit(DumpField::FieldId(SystemModelFieldId::KernelVersion)),
    DumpOptionField::Unit(DumpField::FieldId(SystemModelFieldId::OsRelease)),
    DumpOptionField::Agg(SystemAggField::Stat),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const SYSTEM_ABOUT: &str = "Dump system stats";

/// Generated about message for System dump so supported fields are up-to-date.
static SYSTEM_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {system_fields}

********************** Aggregated fields **********************

* cpu: includes [{agg_cpu_fields}].

* mem: includes [{agg_memory_fields}].

* vm: includes [{agg_vm_fields}].

* stat: includes [{agg_stat_fields}].

* --detail: includes [<agg_field>.*] for each given aggregated field.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

$ below dump system -b "08:30:00" -e "08:30:30" -f datetime vm hostname -O csv

"#,
        about = SYSTEM_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        system_fields = join(enum_iterator::all::<SystemModelFieldId>()),
        agg_cpu_fields = join(SystemAggField::Cpu.expand(false)),
        agg_memory_fields = join(SystemAggField::Mem.expand(false)),
        agg_vm_fields = join(SystemAggField::Vm.expand(false)),
        agg_stat_fields = join(SystemAggField::Stat.expand(false)),
        default_fields = join(DEFAULT_SYSTEM_FIELDS.to_owned()),
    )
});

#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum DiskAggField {
    Read,
    Write,
    Discard,
    FsInfo,
}

impl AggField<SingleDiskModelFieldId> for DiskAggField {
    fn expand(&self, _detail: bool) -> Vec<SingleDiskModelFieldId> {
        use model::SingleDiskModelFieldId::*;

        match self {
            Self::Read => vec![
                ReadBytesPerSec,
                ReadCompleted,
                ReadMerged,
                ReadSectors,
                TimeSpendReadMs,
            ],
            Self::Write => vec![
                WriteBytesPerSec,
                WriteCompleted,
                WriteMerged,
                WriteSectors,
                TimeSpendWriteMs,
            ],
            Self::Discard => vec![
                DiscardBytesPerSec,
                DiscardCompleted,
                DiscardMerged,
                DiscardSectors,
                TimeSpendDiscardMs,
            ],
            Self::FsInfo => vec![DiskUsage, PartitionSize, FilesystemType],
        }
    }
}

pub type DiskOptionField = DumpOptionField<SingleDiskModelFieldId, DiskAggField>;

pub static DEFAULT_DISK_FIELDS: &[DiskOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Unit(DumpField::FieldId(SingleDiskModelFieldId::Name)),
    DumpOptionField::Unit(DumpField::FieldId(
        SingleDiskModelFieldId::DiskTotalBytesPerSec,
    )),
    DumpOptionField::Unit(DumpField::FieldId(SingleDiskModelFieldId::Major)),
    DumpOptionField::Unit(DumpField::FieldId(SingleDiskModelFieldId::Minor)),
    DumpOptionField::Agg(DiskAggField::Read),
    DumpOptionField::Agg(DiskAggField::Write),
    DumpOptionField::Agg(DiskAggField::Discard),
    DumpOptionField::Agg(DiskAggField::FsInfo),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const DISK_ABOUT: &str = "Dump disk stats";

/// Generated about message for System dump so supported fields are up-to-date.
static DISK_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {disk_fields}

********************** Aggregated fields **********************

* read: includes [{agg_read_fields}].

* write: includes [{agg_write_fields}].

* discard: includes [{agg_discard_fields}].

* fs_info: includes [{agg_fsinfo_fields}].

* --detail: no effect.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Simple example:

$ below dump disk -b "08:30:00" -e "08:30:30" -f read write discard -O csv

Output stats for all "nvme0*" matched disk from 08:30:00 to 08:30:30:

$ below dump disk -b "08:30:00" -e "08:30:30" -s name -F nvme0* -O json

Output stats for top 5 read partitions for each time slice from 08:30:00 to 08:30:30:

$ below dump disk -b "08:30:00" -e "08:30:30" -s read_bytes_per_sec --rsort --top 5

"#,
        about = DISK_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        disk_fields = join(enum_iterator::all::<SingleDiskModelFieldId>()),
        agg_read_fields = join(DiskAggField::Read.expand(false)),
        agg_write_fields = join(DiskAggField::Write.expand(false)),
        agg_discard_fields = join(DiskAggField::Discard.expand(false)),
        agg_fsinfo_fields = join(DiskAggField::FsInfo.expand(false)),
        default_fields = join(DEFAULT_DISK_FIELDS.to_owned()),
    )
});

#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum BtrfsAggField {
    DiskUsage,
}

impl AggField<BtrfsModelFieldId> for BtrfsAggField {
    fn expand(&self, _detail: bool) -> Vec<BtrfsModelFieldId> {
        use model::BtrfsModelFieldId::*;

        match self {
            Self::DiskUsage => vec![DiskFraction, DiskBytes],
        }
    }
}

pub type BtrfsOptionField = DumpOptionField<BtrfsModelFieldId, BtrfsAggField>;

pub static DEFAULT_BTRFS_FIELDS: &[BtrfsOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Unit(DumpField::FieldId(BtrfsModelFieldId::Name)),
    DumpOptionField::Agg(BtrfsAggField::DiskUsage),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const BTRFS_ABOUT: &str = "Dump btrfs Stats";

static BTRFS_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {btrfs_fields}

********************** Aggregated fields **********************

* usage: includes [{agg_disk_usage_fields}].

* --detail: no effect.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Simple example:

$ below dump btrfs -b "08:30:00" -e "08:30:30" -f usage -O csv

Output stats for top 5 subvolumes for each time slice from 08:30:00 to 08:30:30:

$ below dump btrfs -b "08:30:00" -e "08:30:30" -s disk_bytes --rsort --top 5

"#,
        about = BTRFS_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        btrfs_fields = join(enum_iterator::all::<BtrfsModelFieldId>()),
        agg_disk_usage_fields = join(BtrfsAggField::DiskUsage.expand(false)),
        default_fields = join(DEFAULT_BTRFS_FIELDS.to_owned()),
    )
});

/// Represents the four sub-model of ProcessModel.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum ProcessAggField {
    Cpu,
    Mem,
    Io,
}

impl AggField<SingleProcessModelFieldId> for ProcessAggField {
    fn expand(&self, detail: bool) -> Vec<SingleProcessModelFieldId> {
        use model::ProcessCpuModelFieldId as Cpu;
        use model::ProcessIoModelFieldId as Io;
        use model::ProcessMemoryModelFieldId as Mem;
        use model::SingleProcessModelFieldId as FieldId;

        if detail {
            match self {
                Self::Cpu => enum_iterator::all::<Cpu>().map(FieldId::Cpu).collect(),
                Self::Mem => enum_iterator::all::<Mem>().map(FieldId::Mem).collect(),
                Self::Io => enum_iterator::all::<Io>().map(FieldId::Io).collect(),
            }
        } else {
            // Default fields for each group
            match self {
                Self::Cpu => vec![FieldId::Cpu(Cpu::UsagePct)],
                Self::Mem => vec![FieldId::Mem(Mem::RssBytes)],
                Self::Io => vec![FieldId::Io(Io::RbytesPerSec), FieldId::Io(Io::WbytesPerSec)],
            }
        }
    }
}

pub type ProcessOptionField = DumpOptionField<SingleProcessModelFieldId, ProcessAggField>;

pub static DEFAULT_PROCESS_FIELDS: &[ProcessOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::Pid)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::Ppid)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::Comm)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::State)),
    DumpOptionField::Agg(ProcessAggField::Cpu),
    DumpOptionField::Agg(ProcessAggField::Mem),
    DumpOptionField::Agg(ProcessAggField::Io),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::UptimeSecs)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::Cgroup)),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::Cmdline)),
    DumpOptionField::Unit(DumpField::FieldId(SingleProcessModelFieldId::ExePath)),
];

const PROCESS_ABOUT: &str = "Dump process stats";

/// Generated about message for Process dump so supported fields are up-to-date.
static PROCESS_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {process_fields}

********************** Aggregated fields **********************

* cpu: includes [{agg_cpu_fields}].

* mem: includes [{agg_memory_fields}].

* io: includes [{agg_io_fields}].

* --detail: includes [<agg_field>.*] for each given aggregated field.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Simple example:

$ below dump process -b "08:30:00" -e "08:30:30" -f comm cpu io.rwbytes_per_sec -O csv

Output stats for all "below*" matched processes from 08:30:00 to 08:30:30:

$ below dump process -b "08:30:00" -e "08:30:30" -s comm -F below* -O json

Output stats for top 5 CPU intense processes for each time slice from 08:30:00 to 08:30:30:

$ below dump process -b "08:30:00" -e "08:30:30" -s cpu.usage_pct --rsort --top 5

"#,
        about = PROCESS_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        process_fields = join(enum_iterator::all::<SingleProcessModelFieldId>()),
        agg_cpu_fields = join(ProcessAggField::Cpu.expand(false)),
        agg_memory_fields = join(ProcessAggField::Mem.expand(false)),
        agg_io_fields = join(ProcessAggField::Io.expand(false)),
        default_fields = join(DEFAULT_PROCESS_FIELDS.to_owned()),
    )
});

/// Represents the four sub-model of SingleCgroupModel.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum CgroupAggField {
    Cpu,
    Mem,
    Io,
    Pressure,
}

impl AggField<SingleCgroupModelFieldId> for CgroupAggField {
    fn expand(&self, detail: bool) -> Vec<SingleCgroupModelFieldId> {
        use model::CgroupCpuModelFieldId as Cpu;
        use model::CgroupIoModelFieldId as Io;
        use model::CgroupMemoryModelFieldId as Mem;
        use model::CgroupPressureModelFieldId as Pressure;
        use model::SingleCgroupModelFieldId as FieldId;

        if detail {
            match self {
                Self::Cpu => enum_iterator::all::<Cpu>().map(FieldId::Cpu).collect(),
                Self::Mem => enum_iterator::all::<Mem>().map(FieldId::Mem).collect(),
                Self::Io => enum_iterator::all::<Io>().map(FieldId::Io).collect(),
                Self::Pressure => enum_iterator::all::<Pressure>()
                    .map(FieldId::Pressure)
                    .collect(),
            }
        } else {
            // Default fields for each group
            match self {
                Self::Cpu => vec![FieldId::Cpu(Cpu::UsagePct)],
                Self::Mem => vec![FieldId::Mem(Mem::Total)],
                Self::Io => vec![FieldId::Io(Io::RbytesPerSec), FieldId::Io(Io::WbytesPerSec)],
                Self::Pressure => vec![
                    FieldId::Pressure(Pressure::CpuSomePct),
                    FieldId::Pressure(Pressure::MemoryFullPct),
                    FieldId::Pressure(Pressure::IoFullPct),
                ],
            }
        }
    }
}

pub type CgroupOptionField = DumpOptionField<SingleCgroupModelFieldId, CgroupAggField>;

pub static DEFAULT_CGROUP_FIELDS: &[CgroupOptionField] = &[
    DumpOptionField::Unit(DumpField::FieldId(SingleCgroupModelFieldId::Name)),
    DumpOptionField::Unit(DumpField::FieldId(SingleCgroupModelFieldId::InodeNumber)),
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Agg(CgroupAggField::Cpu),
    DumpOptionField::Agg(CgroupAggField::Mem),
    DumpOptionField::Agg(CgroupAggField::Io),
    DumpOptionField::Agg(CgroupAggField::Pressure),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const CGROUP_ABOUT: &str = "Dump cgroup stats";

/// Generated about message for Cgroup dump so supported fields are up-to-date.
static CGROUP_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {cgroup_fields}

********************** Aggregated fields **********************

* cpu: includes [{agg_cpu_fields}].

* mem: includes [{agg_memory_fields}].

* io: includes [{agg_io_fields}].

* pressure: includes [{agg_pressure_fields}].

* --detail: includes [<agg_field>.*] for each given aggregated field.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Simple example:

$ below dump cgroup -b "08:30:00" -e "08:30:30" -f name cpu -O csv

Output stats for all cgroups matching pattern "below*" for time slices
from 08:30:00 to 08:30:30:

$ below dump cgroup -b "08:30:00" -e "08:30:30" -s name -F below* -O json

Output stats for top 5 CPU intense cgroups for each time slice
from 08:30:00 to 08:30:30 recursively:

$ below dump cgroup -b "08:30:00" -e "08:30:30" -s cpu.usage_pct --rsort --top 5

"#,
        about = CGROUP_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        cgroup_fields = join(enum_iterator::all::<SingleCgroupModelFieldId>()),
        agg_cpu_fields = join(CgroupAggField::Cpu.expand(false)),
        agg_memory_fields = join(CgroupAggField::Mem.expand(false)),
        agg_io_fields = join(CgroupAggField::Io.expand(false)),
        agg_pressure_fields = join(CgroupAggField::Pressure.expand(false)),
        default_fields = join(DEFAULT_CGROUP_FIELDS.to_owned()),
    )
});

/// Represents the iface sub-models of network model.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum IfaceAggField {
    Rate,
    Rx,
    Tx,
}

impl AggField<SingleNetModelFieldId> for IfaceAggField {
    fn expand(&self, _detail: bool) -> Vec<SingleNetModelFieldId> {
        use model::SingleNetModelFieldId::*;
        match self {
            Self::Rate => vec![
                RxBytesPerSec,
                TxBytesPerSec,
                ThroughputPerSec,
                RxPacketsPerSec,
                TxPacketsPerSec,
            ],
            Self::Rx => vec![
                RxBytes,
                RxCompressed,
                RxCrcErrors,
                RxDropped,
                RxErrors,
                RxFifoErrors,
                RxFrameErrors,
                RxLengthErrors,
                RxMissedErrors,
                RxNohandler,
                RxOverErrors,
                RxPackets,
            ],
            Self::Tx => vec![
                TxAbortedErrors,
                TxBytes,
                TxCarrierErrors,
                TxCompressed,
                TxDropped,
                TxErrors,
                TxFifoErrors,
                TxHeartbeatErrors,
                TxPackets,
                TxWindowErrors,
            ],
        }
    }
}

pub type IfaceOptionField = DumpOptionField<SingleNetModelFieldId, IfaceAggField>;

pub static DEFAULT_IFACE_FIELDS: &[IfaceOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Unit(DumpField::FieldId(SingleNetModelFieldId::Collisions)),
    DumpOptionField::Unit(DumpField::FieldId(SingleNetModelFieldId::Multicast)),
    DumpOptionField::Unit(DumpField::FieldId(SingleNetModelFieldId::Interface)),
    DumpOptionField::Agg(IfaceAggField::Rate),
    DumpOptionField::Agg(IfaceAggField::Rx),
    DumpOptionField::Agg(IfaceAggField::Tx),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const IFACE_ABOUT: &str = "Dump the link layer iface stats";

/// Generated about message for Iface dump so supported fields are up-to-date.
static IFACE_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {iface_fields}

********************** Aggregated fields **********************

* rate: includes [{agg_rate_fields}].

* rx: includes [{agg_rx_fields}].

* tx: includes [{agg_tx_fields}].

* --detail: no effect.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Simple example:

$ below dump iface -b "08:30:00" -e "08:30:30" -f interface rate -O csv

Output stats for all iface stats matching pattern "eth*" for time slices
from 08:30:00 to 08:30:30:

$ below dump iface -b "08:30:00" -e "08:30:30" -s interface -F eth* -O json

"#,
        about = IFACE_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        iface_fields = join(enum_iterator::all::<SingleNetModelFieldId>()),
        agg_rate_fields = join(IfaceAggField::Rate.expand(false)),
        agg_rx_fields = join(IfaceAggField::Rx.expand(false)),
        agg_tx_fields = join(IfaceAggField::Tx.expand(false)),
        default_fields = join(DEFAULT_IFACE_FIELDS.to_owned()),
    )
});

/// Represents the ip and icmp sub-models of the network model.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum NetworkAggField {
    Ip,
    Ip6,
    Icmp,
    Icmp6,
}

impl AggField<NetworkModelFieldId> for NetworkAggField {
    fn expand(&self, _detail: bool) -> Vec<NetworkModelFieldId> {
        use model::NetworkModelFieldId as FieldId;
        match self {
            Self::Ip => enum_iterator::all::<model::IpModelFieldId>()
                .map(FieldId::Ip)
                .collect(),
            Self::Ip6 => enum_iterator::all::<model::Ip6ModelFieldId>()
                .map(FieldId::Ip6)
                .collect(),
            Self::Icmp => enum_iterator::all::<model::IcmpModelFieldId>()
                .map(FieldId::Icmp)
                .collect(),
            Self::Icmp6 => enum_iterator::all::<model::Icmp6ModelFieldId>()
                .map(FieldId::Icmp6)
                .collect(),
        }
    }
}

pub type NetworkOptionField = DumpOptionField<NetworkModelFieldId, NetworkAggField>;

pub static DEFAULT_NETWORK_FIELDS: &[NetworkOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Agg(NetworkAggField::Ip),
    DumpOptionField::Agg(NetworkAggField::Ip6),
    DumpOptionField::Agg(NetworkAggField::Icmp),
    DumpOptionField::Agg(NetworkAggField::Icmp6),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const NETWORK_ABOUT: &str = "Dump the network layer stats including ip and icmp";

/// Generated about message for Network dump so supported fields are up-to-date.
static NETWORK_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {network_fields}

********************** Aggregated fields **********************

* ip: includes [{agg_ip_fields}].

* ip6: includes [{agg_ip6_fields}].

* icmp: includes [{agg_icmp_fields}].

* icmp6: includes [{agg_icmp6_fields}].

* --detail: no effect.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Example:

$ below dump network -b "08:30:00" -e "08:30:30" -f ip ip6 -O json

"#,
        about = NETWORK_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        network_fields = join(enum_iterator::all::<NetworkModelFieldId>()),
        agg_ip_fields = join(NetworkAggField::Ip.expand(false)),
        agg_ip6_fields = join(NetworkAggField::Ip6.expand(false)),
        agg_icmp_fields = join(NetworkAggField::Icmp.expand(false)),
        agg_icmp6_fields = join(NetworkAggField::Icmp6.expand(false)),
        default_fields = join(DEFAULT_NETWORK_FIELDS.to_owned()),
    )
});

/// Represents the tcp and udp sub-models of the network model.
#[derive(
    Clone,
    Debug,
    PartialEq,
    below_derive::EnumFromStr,
    below_derive::EnumToString
)]
pub enum TransportAggField {
    Tcp,
    Udp,
    Udp6,
}

impl AggField<NetworkModelFieldId> for TransportAggField {
    fn expand(&self, _detail: bool) -> Vec<NetworkModelFieldId> {
        use model::NetworkModelFieldId as FieldId;
        match self {
            Self::Tcp => enum_iterator::all::<model::TcpModelFieldId>()
                .map(FieldId::Tcp)
                .collect(),
            Self::Udp => enum_iterator::all::<model::UdpModelFieldId>()
                .map(FieldId::Udp)
                .collect(),
            Self::Udp6 => enum_iterator::all::<model::Udp6ModelFieldId>()
                .map(FieldId::Udp6)
                .collect(),
        }
    }
}

pub type TransportOptionField = DumpOptionField<NetworkModelFieldId, TransportAggField>;

pub static DEFAULT_TRANSPORT_FIELDS: &[TransportOptionField] = &[
    DumpOptionField::Unit(DumpField::Common(CommonField::Datetime)),
    DumpOptionField::Agg(TransportAggField::Tcp),
    DumpOptionField::Agg(TransportAggField::Udp),
    DumpOptionField::Agg(TransportAggField::Udp6),
    DumpOptionField::Unit(DumpField::Common(CommonField::Timestamp)),
];

const TRANSPORT_ABOUT: &str = "Dump the transport layer stats including tcp and udp";

/// Generated about message for Transport dump so supported fields are up-to-date.
static TRANSPORT_LONG_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        r#"{about}

********************** Available fields **********************

{common_fields}, {network_fields}.

********************** Aggregated fields **********************

* tcp: includes [{agg_tcp_fields}].

* udp: includes [{agg_udp_fields}].

* udp6: includes [{agg_udp6_fields}].

* --detail: no effect.

* --default: includes [{default_fields}].

* --everything: includes everything (equivalent to --default --detail).

********************** Example Commands **********************

Example:

$ below dump transport -b "08:30:00" -e "08:30:30" -f tcp udp -O json

"#,
        about = TRANSPORT_ABOUT,
        common_fields = join(enum_iterator::all::<CommonField>()),
        network_fields = join(enum_iterator::all::<NetworkModelFieldId>()),
        agg_tcp_fields = join(TransportAggField::Tcp.expand(false)),
        agg_udp_fields = join(TransportAggField::Udp.expand(false)),
        agg_udp6_fields = join(TransportAggField::Udp6.expand(false)),
        default_fields = join(DEFAULT_TRANSPORT_FIELDS.to_owned()),
    )
});

make_option! (OutputFormat {
    "raw": Raw,
    "csv": Csv,
    "tsv": Tsv,
    "json": Json,
    "kv": KeyVal,
    "openmetrics": OpenMetrics,
});

#[derive(Debug, Parser, Default, Clone)]
pub struct GeneralOpt {
    /// Show all top layer fields. If --default is specified, it overrides any specified fields via --fields.
    #[clap(long)]
    pub default: bool,
    /// Show all fields. If --everything is specified, --fields and --default are overridden.
    #[clap(long)]
    pub everything: bool,
    /// Show more infomation other than default.
    #[clap(short, long)]
    pub detail: bool,
    /// Begin time, same format as replay
    #[clap(long, short)]
    pub begin: String,
    /// End time, same format as replay
    #[clap(long, short, group = "time")]
    pub end: Option<String>,
    /// Time string specifying the duration, e.g. "10 min"{n}
    /// Keywords: days min, h, sec{n}
    /// Relative: {humantime}, e.g. "2 days 3 hr 15m 10sec"{n}
    /// _
    #[clap(long, group = "time")]
    pub duration: Option<String>,
    /// Take a regex and apply to --select selected field. See command level doc for example.
    #[clap(long, short = 'F')]
    pub filter: Option<Regex>,
    /// Sort (lower to higher) by --select selected field. See command level doc for example.
    #[clap(long)]
    pub sort: bool,
    /// Sort (higher to lower) by --select selected field. See command level doc for example.
    #[clap(long)]
    pub rsort: bool,
    // display top N field. See command level doc for example.
    #[clap(long, default_value = "0")]
    pub top: u32,
    /// Repeat title, for each N line, it will render a line of title. Only for raw output format.
    #[clap(long = "repeat-title")]
    pub repeat_title: Option<usize>,
    /// Output format. Choose from raw, csv, tsv, kv, json, openmetrics. Default to raw
    #[clap(long, short = 'O')]
    pub output_format: Option<OutputFormat>,
    /// Output destination, default to stdout.
    #[clap(long, short)]
    pub output: Option<String>,
    /// Disable title in raw, csv or tsv format output
    #[clap(long)]
    pub disable_title: bool,
    /// Days adjuster, same as -r option in replay.
    #[clap(short = 'r')]
    pub yesterdays: Option<String>,
    /// Line break symbol between samples
    #[clap(long)]
    pub br: Option<String>,
    /// Dump raw data without units or conversion
    #[clap(long)]
    pub raw: bool,
}

#[derive(Debug, Parser, Clone)]
pub enum DumpCommand {
    #[clap(about = SYSTEM_ABOUT, long_about = SYSTEM_LONG_ABOUT.as_str())]
    System {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<SystemOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Saved pattern in the dumprc file under [system] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = DISK_ABOUT, long_about = DISK_LONG_ABOUT.as_str())]
    Disk {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<DiskOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Select field for operation, use with --sort, --rsort, --filter, --top
        #[clap(long, short)]
        select: Option<SingleDiskModelFieldId>,
        /// Saved pattern in the dumprc file under [disk] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = BTRFS_ABOUT, long_about = BTRFS_LONG_ABOUT.as_str())]
    Btrfs {
        /// Select which fields to display and in what order.
        #[clap(short, long)]
        fields: Option<Vec<BtrfsOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Select field for operation, use with --sort, --rsort, --filter, --top
        #[clap(long, short)]
        select: Option<BtrfsModelFieldId>,
        /// Saved pattern in the dumprc file under [btrfs] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = PROCESS_ABOUT, long_about = PROCESS_LONG_ABOUT.as_str())]
    Process {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<ProcessOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Select field for operation, use with --sort, --rsort, --filter, --top
        #[clap(long, short)]
        select: Option<SingleProcessModelFieldId>,
        /// Saved pattern in the dumprc file under [process] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = CGROUP_ABOUT, long_about = CGROUP_LONG_ABOUT.as_str())]
    Cgroup {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<CgroupOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Select field for operation, use with --sort, --rsort, --filter, --top
        #[clap(long, short)]
        select: Option<SingleCgroupModelFieldId>,
        /// Saved pattern in the dumprc file under [cgroup] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = IFACE_ABOUT, long_about = IFACE_LONG_ABOUT.as_str())]
    Iface {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<IfaceOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Select field for operation, use with --filter
        #[clap(long, short)]
        select: Option<SingleNetModelFieldId>,
        /// Saved pattern in the dumprc file under [iface] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = NETWORK_ABOUT, long_about = NETWORK_LONG_ABOUT.as_str())]
    Network {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<NetworkOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Saved pattern in the dumprc file under [network] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
    #[clap(about = TRANSPORT_ABOUT, long_about = TRANSPORT_LONG_ABOUT.as_str())]
    Transport {
        /// Select which fields to display and in what order.
        #[clap(short, long, num_args = 1..)]
        fields: Option<Vec<TransportOptionField>>,
        #[clap(flatten)]
        opts: GeneralOpt,
        /// Saved pattern in the dumprc file under [transport] section.
        #[clap(long, short, conflicts_with("fields"))]
        pattern: Option<String>,
    },
}
