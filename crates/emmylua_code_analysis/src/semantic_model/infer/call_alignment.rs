//! Which declared param each argument of a call meets (`CallAlignment`): one rule for the
//! checkers (`check/checker/param_count.rs` and the rest) and for inference (`vm.rs`'s closure
//! arguments, #77).
use crate::{LuaFunctionType, LuaType};

/// Which declared param each argument of a call meets: the receiver shift, decided once per
/// candidate from the call's syntax and the definition. Every site that pairs arguments with
/// params reads it (`docs/agents/checkers.md`, "Argument/param alignment"): a hand-written
/// offset was #76's bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallAlignment {
    /// Argument `i` fills param `i`: a dot call to a plain function or to one whose params name
    /// `self`, or a colon call to a colon-defined method (whose params hold no `self`).
    Plain,
    /// `o:m(x)` where the first param is `self` (named `self`, `fun(self: T)`, `---@param x
    /// self`): the receiver fills it, and argument `i` fills param `i + 1`.
    ReceiverIsSelf,
    /// `o:f(x)` on a plain `function M.f(a, x)`: the receiver fills param 0 as an argument would
    /// (its type is checked against it), and argument `i` fills param `i + 1`.
    ReceiverIsParam0,
    /// `M.m(o, x)` on `function M:m(x)`: argument 0 is the receiver and fills no declared param,
    /// and argument `i` fills param `i - 1`.
    ArgIsReceiver,
}

impl CallAlignment {
    pub(crate) fn of(func: &LuaFunctionType, colon_call: bool) -> Self {
        match (
            colon_call,
            first_param_is_self(func),
            func.is_colon_define(),
        ) {
            (true, true, _) => Self::ReceiverIsSelf,
            (true, false, false) => Self::ReceiverIsParam0,
            (false, false, true) => Self::ArgIsReceiver,
            _ => Self::Plain,
        }
    }

    /// The declared params the receiver fills, before the one argument 0 fills.
    pub(crate) fn param_skip(self) -> usize {
        usize::from(matches!(
            self,
            Self::ReceiverIsSelf | Self::ReceiverIsParam0
        ))
    }

    /// The arguments that fill no declared param, before the one param 0 takes: a dot call's
    /// receiver.
    pub(crate) fn arg_skip(self) -> usize {
        usize::from(self == Self::ArgIsReceiver)
    }

    /// The declared param argument `arg_index` fills; `None` for `ArgIsReceiver`'s receiver.
    pub(crate) fn param_of(self, arg_index: usize) -> Option<usize> {
        (arg_index + self.param_skip()).checked_sub(self.arg_skip())
    }

    /// The declared type of the param argument `arg_index` fills; `None` for `ArgIsReceiver`'s
    /// receiver, past the params, or for an untyped param.
    pub(crate) fn param(
        self,
        params: &[(String, Option<LuaType>)],
        arg_index: usize,
    ) -> Option<&LuaType> {
        params.get(self.param_of(arg_index)?)?.1.as_ref()
    }

    /// `ReceiverIsParam0`'s first param, typed: the one the receiver is checked against.
    pub(crate) fn receiver_param(
        self,
        params: &[(String, Option<LuaType>)],
    ) -> Option<(&String, &LuaType)> {
        if self != Self::ReceiverIsParam0 {
            return None;
        }
        let (name, ty) = params.first()?;
        Some((name, ty.as_ref()?))
    }

    /// The declared params the explicit arguments meet, in order: those after the receiver's.
    pub(crate) fn explicit_params<P>(self, params: &[P]) -> &[P] {
        params.get(self.param_skip()..).unwrap_or(&[])
    }

    /// The arguments that meet declared params, in order: those after a dot call's receiver
    /// (`args`, or their cached types).
    pub(crate) fn explicit_args<T>(self, args: &[T]) -> &[T] {
        args.get(self.arg_skip()..).unwrap_or(&[])
    }
}

/// Whether `func`'s first param is the receiver's `self`: named `self`, or typed `self`.
pub(crate) fn first_param_is_self(func: &LuaFunctionType) -> bool {
    func.get_params().first().is_some_and(|(name, ty)| {
        name == "self"
            || ty.as_ref().is_some_and(|t| {
                t.is_self_infer() || matches!(t, LuaType::Ref(id) if id.get_name() == "self")
            })
    })
}
