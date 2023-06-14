use super::{sys, ArgCmp, Rule};
use std::ffi::c_int;

pub(crate) struct RawRule {
    pub(crate) action: u32,
    pub(crate) syscall: c_int,
    pub(crate) arg_cmps: Vec<sys::scmp_arg_cmp>,
}

impl From<&Rule> for RawRule {
    fn from(rule: &Rule) -> RawRule {
        let (action, syscall, arg_cmps) = match rule {
            Rule::Allow(syscall, arg_cmps) => (sys::SCMP_ACT_ALLOW, syscall, arg_cmps),
            Rule::AllowAndLog(syscall, arg_cmps) => (sys::SCMP_ACT_LOG, syscall, arg_cmps),
            Rule::ReturnError(syscall, err_num, arg_cmps) => {
                //NOTE: https://github.com/seccomp/libseccomp/blob/f1c3196d9b95de22dde8f23c5befcbeabef5711c/include/seccomp.h.in#L377
                (0x50000 | (*err_num as u32 & 0xffff), syscall, arg_cmps)
            }
        };

        RawRule {
            action,
            syscall: *syscall as i32,
            arg_cmps: arg_cmps.iter().map(Into::into).collect(),
        }
    }
}

impl From<&ArgCmp> for sys::scmp_arg_cmp {
    fn from(arg_cmp: &ArgCmp) -> Self {
        macro_rules! to_sys_op {
            ( $( $Op:ident => $sys_op:ident ),+ ) => {
                match arg_cmp {
                    $(ArgCmp::$Op { arg_idx, value } => sys::scmp_arg_cmp {
                        arg: *arg_idx,
                        op: sys::$sys_op,
                        datum_a: value.0,
                        datum_b: 0,
                    },)+
                    ArgCmp::EqualMasked { arg_idx, mask, value } => sys::scmp_arg_cmp {
                        arg: *arg_idx,
                        op: sys::scmp_compare_SCMP_CMP_MASKED_EQ,
                        datum_a: *mask,
                        datum_b: value.0,
                    }
                }
            }
        }

        to_sys_op!(
            NotEqual => scmp_compare_SCMP_CMP_NE,
            LessThan => scmp_compare_SCMP_CMP_LT,
            LessThanOrEqual => scmp_compare_SCMP_CMP_LE,
            Equal => scmp_compare_SCMP_CMP_EQ,
            GreaterThanOrEqual => scmp_compare_SCMP_CMP_GE,
            GreaterThan => scmp_compare_SCMP_CMP_GT
        )
    }
}
