use super::*;
use kdl_script::parse::{Attr, AttrAligned, AttrPacked, AttrPassthrough, AttrRepr, LangRepr, Repr};
use kdl_script::types::{AliasTy, ArrayTy, FuncIdx, PrimitiveTy, RefTy, Ty, TyIdx};
use std::fmt::Write;

impl CcAbiImpl {
    pub fn generate_caller_externs(
        &self,
        f: &mut Fivemat,
        state: &TestState,
    ) -> Result<(), GenerateError> {
        let convention_decl = self.convention_decl(state.options.convention)?;
        writeln!(f, "extern \"{convention_decl}\" {{",)?;
        f.add_indent(1);
        for &func in &state.desired_funcs {
            self.generate_signature(f, state, func)?;
            writeln!(f, ";")?;
        }
        f.sub_indent(1);
        writeln!(f, "}}")?;
        writeln!(f)?;
        Ok(())
    }

    pub fn generate_definitions(
        &self,
        f: &mut Fivemat,
        state: &mut TestState,
    ) -> Result<(), GenerateError> {
        self.write_harness_prefix(f, state)?;

        for def in state.defs.definitions(state.desired_funcs.iter().copied()) {
            match def {
                kdl_script::Definition::DeclareTy(ty) => {
                    debug!("declare ty {}", state.types.format_ty(ty));
                    self.generate_forward_decl(f, state, ty)?;
                }
                kdl_script::Definition::DefineTy(ty) => {
                    debug!("define ty {}", state.types.format_ty(ty));
                    self.generate_tydef(f, state, ty)?;
                }
                kdl_script::Definition::DefineFunc(_) => {
                    // we'd buffer these up to generate them all at the end,
                    // but we've already got them buffered, so... do nothing.
                }
                kdl_script::Definition::DeclareFunc(_) => {
                    // nothing to do, executable kdl-script isn't real and can't hurt us
                }
            }
        }

        Ok(())
    }

    pub fn intern_tyname(&self, state: &mut TestState, ty: TyIdx) -> Result<(), GenerateError> {
        // Don't double-intern
        if state.tynames.contains_key(&ty) {
            return Ok(());
        }

        let (prefix, suffix) = match state.types.realize_ty(ty) {
            // Structural types that don't need definitions but we should
            // intern the name of
            Ty::Primitive(prim) => {
                let name = match prim {
                    PrimitiveTy::I8 => "int8_t ",
                    PrimitiveTy::I16 => "int16_t ",
                    PrimitiveTy::I32 => "int32_4 ",
                    PrimitiveTy::I64 => "int64_t ",
                    PrimitiveTy::I128 => "__int128_t ",
                    PrimitiveTy::U8 => "uint8_t ",
                    PrimitiveTy::U16 => "uint16_t ",
                    PrimitiveTy::U32 => "uint32_t ",
                    PrimitiveTy::U64 => "uint64_t ",
                    PrimitiveTy::U128 => "__uint128_t ",
                    PrimitiveTy::F32 => "float ",
                    PrimitiveTy::F64 => "double ",
                    PrimitiveTy::Bool => "bool ",
                    PrimitiveTy::Ptr => "void *",
                    PrimitiveTy::I256 => {
                        Err(UnsupportedError::Other("c doesn't have i256?".to_owned()))?
                    }
                    PrimitiveTy::U256 => {
                        Err(UnsupportedError::Other("c doesn't have u256?".to_owned()))?
                    }
                    PrimitiveTy::F16 => {
                        Err(UnsupportedError::Other("c doesn't have f16?".to_owned()))?
                    }
                    PrimitiveTy::F128 => {
                        Err(UnsupportedError::Other("c doesn't have f128?".to_owned()))?
                    }
                };
                (name.to_owned(), None)
            }
            Ty::Array(ArrayTy { elem_ty, len }) => {
                let (pre, post) = &state.tynames[elem_ty];
                (pre.clone(), Some(format!("[{len}]{post}")))
            }
            Ty::Ref(RefTy { pointee_ty }) => {
                let (pre, post) = &state.tynames[pointee_ty];
                // If the last type modifier was postfix (an array dimension)
                // Then we need to introduce a set of parens to make this pointer
                // bind more tightly
                let was_postfix = matches!(state.types.realize_ty(*pointee_ty), Ty::Array(_));
                if was_postfix {
                    (format!("{pre}(*"), Some(format!("){post}")))
                } else {
                    (format!("{pre}*"), Some(post.clone()))
                }
            }
            // Nominal types we need to emit a decl for
            Ty::Struct(struct_ty) => (struct_ty.name.to_string(), None),
            Ty::Union(union_ty) => (union_ty.name.to_string(), None),
            Ty::Enum(enum_ty) => (enum_ty.name.to_string(), None),
            Ty::Tagged(tagged_ty) => (tagged_ty.name.to_string(), None),
            Ty::Alias(alias_ty) => (alias_ty.name.to_string(), None),
            // Puns should be evaporated
            Ty::Pun(pun) => {
                let real_ty = state.types.resolve_pun(pun, &state.env).unwrap();
                let (pre, post) = state.tynames[&real_ty].clone();
                (pre, Some(post))
            }
            Ty::Empty => {
                return Err(UnsupportedError::Other(
                    "c doesn't have empty tuples".to_owned(),
                ))?
            }
        };

        state
            .tynames
            .insert(ty, (prefix, suffix.unwrap_or_default()));

        Ok(())
    }

    pub fn generate_forward_decl(
        &self,
        f: &mut Fivemat,
        state: &mut TestState,
        ty: TyIdx,
    ) -> Result<(), GenerateError> {
        // Make sure our own name is interned
        self.intern_tyname(state, ty)?;

        match state.types.realize_ty(ty) {
            // Nominal types we need to emit a decl for
            Ty::Struct(struct_ty) => {
                let ty_name = &struct_ty.name;
                writeln!(f, "typedef struct {ty_name} {ty_name};")?;
            }
            Ty::Union(union_ty) => {
                let ty_name = &union_ty.name;
                writeln!(f, "typedef union {ty_name} {ty_name};")?;
            }
            Ty::Enum(enum_ty) => {
                let ty_name = &enum_ty.name;
                writeln!(f, "typedef enum {ty_name} {ty_name};")?;
            }
            Ty::Tagged(tagged_ty) => {
                let ty_name = &tagged_ty.name;
                writeln!(f, "typedef struct {ty_name} {ty_name};")?;
            }
            Ty::Alias(AliasTy { name, real, attrs }) => {
                if !attrs.is_empty() {
                    return Err(UnsupportedError::Other(
                        "don't yet know how to apply attrs to aliases".to_string(),
                    ))?;
                }
                let (pre, post) = &state.tynames[real];
                writeln!(f, "typedef {pre}{name}{post};\n")?;
            }
            Ty::Pun(..) => {
                // Puns should be evaporated by the type name interner
            }
            Ty::Primitive(prim) => {
                match prim {
                    PrimitiveTy::I8
                    | PrimitiveTy::I16
                    | PrimitiveTy::I32
                    | PrimitiveTy::I64
                    | PrimitiveTy::I128
                    | PrimitiveTy::I256
                    | PrimitiveTy::U8
                    | PrimitiveTy::U16
                    | PrimitiveTy::U32
                    | PrimitiveTy::U64
                    | PrimitiveTy::U128
                    | PrimitiveTy::U256
                    | PrimitiveTy::F16
                    | PrimitiveTy::F32
                    | PrimitiveTy::F64
                    | PrimitiveTy::F128
                    | PrimitiveTy::Bool
                    | PrimitiveTy::Ptr => {
                        // Builtin
                    }
                };
            }
            Ty::Array(ArrayTy { .. }) => {
                // Builtin
            }
            Ty::Ref(RefTy { .. }) => {
                // Builtin
            }
            Ty::Empty => {
                return Err(UnsupportedError::Other(
                    "c doesn't have empty tuples".to_owned(),
                ))?;
            }
        }
        Ok(())
    }

    pub fn generate_tydef(
        &self,
        f: &mut Fivemat,
        state: &mut TestState,
        ty: TyIdx,
    ) -> Result<(), GenerateError> {
        // Make sure our own name is interned
        self.intern_tyname(state, ty)?;

        match state.types.realize_ty(ty) {
            // Nominal types we need to emit a decl for
            Ty::Struct(struct_ty) => {
                // Emit an actual struct decl
                self.generate_repr_attr(f, &struct_ty.attrs, "struct")?;
                writeln!(f, "typedef struct {} {{", struct_ty.name)?;
                f.add_indent(1);
                for field in &struct_ty.fields {
                    let field_name = &field.ident;
                    let (pre, post) = &state.tynames[&field.ty];
                    writeln!(f, "{pre}{field_name}{post};")?;
                }
                f.sub_indent(1);
                writeln!(f, "}} {};\n", struct_ty.name)?;
            }
            Ty::Union(union_ty) => {
                // Emit an actual union decl
                self.generate_repr_attr(f, &union_ty.attrs, "union")?;
                writeln!(f, "typedef union {} {{", union_ty.name)?;
                f.add_indent(1);
                for field in &union_ty.fields {
                    let field_name = &field.ident;
                    let (pre, post) = &state.tynames[&field.ty];
                    writeln!(f, "{pre}{field_name}{post};")?;
                }
                f.sub_indent(1);
                writeln!(f, "}} {};\n", union_ty.name)?;
            }
            Ty::Enum(enum_ty) => {
                // Emit an actual enum decl
                self.generate_repr_attr(f, &enum_ty.attrs, "enum")?;
                writeln!(f, "typedef enum {} {{", enum_ty.name)?;
                f.add_indent(1);
                for variant in &enum_ty.variants {
                    let variant_name = &variant.name;
                    writeln!(f, "{variant_name};")?;
                }
                f.sub_indent(1);
                writeln!(f, "}} {};\n", enum_ty.name)?;
            }
            Ty::Tagged(tagged_ty) => {
                return Err(UnsupportedError::Other(
                    "c doesn't have tagged unions impled yet".to_owned(),
                ))?;
                /*
                // Emit an actual enum decl
                self.generate_repr_attr(f, &tagged_ty.attrs, "tagged")?;
                writeln!(f, "typedef struct {} {{", tagged_ty.name)?;
                f.add_indent(1);
                for variant in &tagged_ty.variants {
                    let variant_name = &variant.name;
                    if let Some(fields) = &variant.fields {
                        writeln!(f, "{variant_name} {{")?;
                        f.add_indent(1);
                        for field in fields {
                            let field_name = &field.ident;
                            let field_tyname = state
                                .borrowed_tynames
                                .get(&field.ty)
                                .unwrap_or(&state.tynames[&field.ty]);
                            writeln!(f, "{field_name}: {field_tyname},")?;
                        }
                        f.sub_indent(1);
                        writeln!(f, "}},")?;
                    } else {
                        writeln!(f, "{variant_name},")?;
                    }
                }
                f.sub_indent(1);
                writeln!(f, "}} {};\n", tagged_ty.name)?;
                 */
            }
            Ty::Alias(_) => {
                // Just reuse the other impl
                self.generate_forward_decl(f, state, ty)?;
            }
            Ty::Pun(..) => {
                // Puns should be evaporated by the type name interner
            }
            Ty::Primitive(prim) => {
                match prim {
                    PrimitiveTy::I8
                    | PrimitiveTy::I16
                    | PrimitiveTy::I32
                    | PrimitiveTy::I64
                    | PrimitiveTy::I128
                    | PrimitiveTy::I256
                    | PrimitiveTy::U8
                    | PrimitiveTy::U16
                    | PrimitiveTy::U32
                    | PrimitiveTy::U64
                    | PrimitiveTy::U128
                    | PrimitiveTy::U256
                    | PrimitiveTy::F16
                    | PrimitiveTy::F32
                    | PrimitiveTy::F64
                    | PrimitiveTy::F128
                    | PrimitiveTy::Bool
                    | PrimitiveTy::Ptr => {
                        // Builtin
                    }
                };
            }
            Ty::Array(ArrayTy { .. }) => {
                // Builtin
            }
            Ty::Ref(RefTy { .. }) => {
                // Builtin
            }
            Ty::Empty => {
                return Err(UnsupportedError::Other(
                    "c doesn't have empty tuples".to_owned(),
                ))?;
            }
        }
        Ok(())
    }

    pub fn generate_repr_attr(
        &self,
        f: &mut Fivemat,
        attrs: &[Attr],
        _ty_style: &str,
    ) -> Result<(), GenerateError> {
        if !attrs.is_empty() {
            return Err(UnsupportedError::Other(
                "c doesn't support attrs yet".to_owned(),
            ))?;
        }

        /*
        let mut default_c_repr = true;
        let mut repr_attrs = vec![];
        let mut other_attrs = vec![];
        for attr in attrs {
            match attr {
                Attr::Align(AttrAligned { align }) => {
                    repr_attrs.push(format!("align({})", align.val));
                }
                Attr::Packed(AttrPacked {}) => {
                    repr_attrs.push("packed".to_owned());
                }
                Attr::Passthrough(AttrPassthrough(attr)) => {
                    other_attrs.push(attr.to_string());
                }
                Attr::Repr(AttrRepr { reprs }) => {
                    // Any explicit repr attributes disables default C
                    default_c_repr = false;
                    for repr in reprs {
                        let val = match repr {
                            Repr::Primitive(prim) => match prim {
                                PrimitiveTy::I8 => "i8",
                                PrimitiveTy::I16 => "i16",
                                PrimitiveTy::I32 => "i32",
                                PrimitiveTy::I64 => "i64",
                                PrimitiveTy::I128 => "i128",
                                PrimitiveTy::U8 => "u8",
                                PrimitiveTy::U16 => "u16",
                                PrimitiveTy::U32 => "u32",
                                PrimitiveTy::U64 => "u64",
                                PrimitiveTy::U128 => "u128",
                                PrimitiveTy::I256
                                | PrimitiveTy::U256
                                | PrimitiveTy::F16
                                | PrimitiveTy::F32
                                | PrimitiveTy::F64
                                | PrimitiveTy::F128
                                | PrimitiveTy::Bool
                                | PrimitiveTy::Ptr => {
                                    return Err(UnsupportedError::Other(format!(
                                        "unsupport repr({prim:?})"
                                    )))?;
                                }
                            },
                            Repr::Lang(LangRepr::C) => "C",
                            Repr::Lang(LangRepr::Rust) => {
                                continue;
                            }
                            Repr::Transparent => "transparent",
                        };
                        repr_attrs.push(val.to_owned());
                    }
                }
            }
        }
        if default_c_repr {
            repr_attrs.push("C".to_owned());
        }
        write!(f, "#[repr(")?;
        let mut multi = false;
        for repr in repr_attrs {
            if multi {
                write!(f, ", ")?;
            }
            multi = true;
            write!(f, "{repr}")?;
        }
        writeln!(f, ")]")?;
        for attr in other_attrs {
            writeln!(f, "{}", attr)?;
        }
        */
        Ok(())
    }

    pub fn generate_signature(
        &self,
        f: &mut Fivemat,
        state: &TestState,
        func: FuncIdx,
    ) -> Result<(), GenerateError> {
        let function = state.types.realize_func(func);

        if let Some(output) = function.outputs.first() {
            let (pre, post) = &state.tynames[&output.ty];
            write!(f, "{pre}{}{post}(", function.name)?;
        } else {
            write!(f, "void {}(", function.name)?;
        }
        let mut multiarg = false;
        // Add inputs
        for arg in &function.inputs {
            if multiarg {
                write!(f, ", ")?;
            }
            multiarg = true;
            let arg_name = &arg.name;
            let (pre, post) = &state.tynames[&arg.ty];
            write!(f, "{pre}{}{post}", arg_name)?;
        }
        Ok(())
    }

    pub fn convention_decl(
        &self,
        convention: CallingConvention,
    ) -> Result<&'static str, GenerateError> {
        use CCFlavor::*;
        use CallingConvention::*;
        use Platform::*;
        // GCC (as __attribute__'s)
        //
        //  * x86: cdecl, fastcall, thiscall, stdcall,
        //         sysv_abi, ms_abi (64-bit: -maccumulate-outgoing-args?),
        //         naked, interrupt, sseregparm
        //  * ARM: pcs="aapcs", pcs="aapcs-vfp",
        //         long_call, short_call, naked,
        //         interrupt("IRQ", "FIQ", "SWI", "ABORT", "UNDEF"),
        //
        // MSVC (as ~keywords)
        //
        //  * __cdecl, __clrcall, __stdcall, __fastcall, __thiscall, __vectorcall

        let val = match convention {
            System | Win64 | Sysv64 | Aapcs => {
                // Don't want to think about these yet, I think they're
                // all properly convered by other ABIs
                return Err(self.unsupported_convention(&convention))?;
            }
            C => "",
            Cdecl => {
                if self.platform == Windows {
                    match self.cc_flavor {
                        Msvc => "__cdecl ",
                        Gcc | Clang => "__attribute__((cdecl)) ",
                    }
                } else {
                    return Err(self.unsupported_convention(&convention))?;
                }
            }
            Stdcall => {
                if self.platform == Windows {
                    match self.cc_flavor {
                        Msvc => "__stdcall ",
                        Gcc | Clang => "__attribute__((stdcall)) ",
                    }
                } else {
                    return Err(self.unsupported_convention(&convention))?;
                }
            }
            Fastcall => {
                if self.platform == Windows {
                    match self.cc_flavor {
                        Msvc => "__fastcall ",
                        Gcc | Clang => "__attribute__((fastcall)) ",
                    }
                } else {
                    return Err(self.unsupported_convention(&convention))?;
                }
            }
            Vectorcall => {
                if self.platform == Windows {
                    match self.cc_flavor {
                        Msvc => "__vectorcall ",
                        Gcc | Clang => "__attribute__((vectorcall)) ",
                    }
                } else {
                    return Err(self.unsupported_convention(&convention))?;
                }
            }
        };

        Ok(val)
    }

    fn unsupported_convention(&self, convention: &CallingConvention) -> UnsupportedError {
        UnsupportedError::Other(format!("unsupported convention {convention}"))
    }
}
