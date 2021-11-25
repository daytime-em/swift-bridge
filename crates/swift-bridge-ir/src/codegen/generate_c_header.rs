use crate::built_in_types::BuiltInType;
use crate::parsed_extern_fn::ParsedExternFn;
use crate::{BridgedType, SharedType, SwiftBridgeModule};
use std::collections::HashSet;
use syn::ReturnType;

const NOTICE: &'static str = "// File automatically generated by swift-bridge.";

struct Bookkeeping {
    includes: HashSet<&'static str>,
    slice_types: HashSet<String>,
}

impl SwiftBridgeModule {
    /// Generate the contents of a C header file based on the contents of this module.
    pub fn generate_c_header(&self) -> String {
        format!(
            r#"{notice}
{header}"#,
            notice = NOTICE,
            header = self.generate_c_header_inner()
        )
    }

    fn generate_c_header_inner(&self) -> String {
        let mut header = "".to_string();

        let mut bookkeeping = Bookkeeping {
            includes: HashSet::new(),
            // TODO: Delete this
            slice_types: HashSet::new(),
        };

        for ty in self.types.iter() {
            match ty {
                BridgedType::Shared(ty) => match ty {
                    SharedType::Struct(ty_struct) => {
                        let name = ty_struct.swift_name_string();

                        let mut fields = vec![];
                        for (idx, field) in ty_struct.fields.iter().enumerate() {
                            let ty = BuiltInType::new_with_type(&field.ty).unwrap();

                            if let Some(include) = ty.c_include() {
                                bookkeeping.includes.insert(include);
                            }

                            let name = format!("_{}", idx);

                            fields.push(format!(
                                "{} {}",
                                ty.to_c(),
                                field.name.as_ref().map(|f| f.to_string()).unwrap_or(name)
                            ));
                        }

                        let maybe_fields = if fields.len() > 0 {
                            let mut maybe_fields = " { ".to_string();

                            maybe_fields += &fields.join("; ");

                            maybe_fields += "; }";
                            maybe_fields
                        } else {
                            "".to_string()
                        };

                        let ty_decl = format!(
                            "typedef struct {name}{maybe_fields} {name};",
                            name = name,
                            maybe_fields = maybe_fields
                        );

                        header += &ty_decl;
                        header += "\n";
                    }
                },
                BridgedType::Opaque(ty) => {
                    if ty.host_lang.is_swift() {
                        continue;
                    }

                    let ty_name = ty.ident.to_string();

                    let ty_decl = format!("typedef struct {ty_name} {ty_name};", ty_name = ty_name);
                    let drop_ty = format!(
                        "void __swift_bridge__${ty_name}$_free(void* self);",
                        ty_name = ty_name
                    );

                    header += &ty_decl;
                    header += "\n";
                    header += &drop_ty;
                    header += "\n";
                }
            }
        }

        for function in self.functions.iter() {
            if function.host_lang.is_swift() {
                continue;
            }

            header += &declare_func(&function, &mut bookkeeping);
        }

        for slice_ty in bookkeeping.slice_types.iter() {
            header = format!(
                r#"typedef struct FfiSlice_{slice_ty} {{ {slice_ty}* start; uintptr_t len; }} FfiSlice_{slice_ty};
{header}"#,
                slice_ty = slice_ty,
                header = header
            )
        }

        let mut includes = bookkeeping.includes.iter().collect::<Vec<_>>();
        includes.sort();
        for include in includes {
            header = format!(
                r#"#include <{}>
{}"#,
                include, header
            );
        }

        header
    }
}

fn declare_func(func: &ParsedExternFn, bookkeeping: &mut Bookkeeping) -> String {
    let ret = func.to_c_header_return();
    let name = func.link_name();
    let params = func.to_c_header_params();

    if let ReturnType::Type(_, ty) = &func.func.sig.output {
        if let Some(ty) = BuiltInType::new_with_type(&ty) {
            if let BuiltInType::RefSlice(ref_slice) = ty {
                bookkeeping.slice_types.insert(ref_slice.ty.to_c());
            }
        }
    }

    if let Some(includes) = func.c_includes() {
        for include in includes {
            bookkeeping.includes.insert(include);
        }
    }

    let declaration = format!(
        "{ret} {name}({params});\n",
        ret = ret,
        name = name,
        params = params
    );

    declaration
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;

    use crate::parse::SwiftBridgeModuleAndErrors;
    use crate::test_utils::assert_generated_equals_expected;
    use crate::SwiftBridgeModule;

    use super::*;

    /// Verify that we generate an empty header file for an empty module.
    #[test]
    fn generates_empty_header_for_empty_section() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" { }
            }
        };
        let module = parse_ok(tokens);

        let header = module.generate_c_header();
        assert_eq!(header.trim(), NOTICE)
    }

    /// Verify that we do not generate any headers for extern "Swift" blocks since Rust does not
    /// need any C headers.
    #[test]
    fn ignores_extern_swift() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Swift" {
                    type Foo;
                    fn bar ();
                }
            }
        };
        let module = parse_ok(tokens);

        let header = module.generate_c_header();
        assert_eq!(header.trim(), NOTICE)
    }

    /// Verify that we generate a type definition for a freestanding function that has no args.
    #[test]
    fn freestanding_function_no_args() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    fn foo();
                }
            }
        };
        let expected = r#"
void __swift_bridge__$foo(void);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a freestanding function that has one arg.
    #[test]
    fn freestanding_function_one_args() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    fn foo(arg1: u8);
                }
            }
        };
        let expected = r#"
#include <stdint.h>
void __swift_bridge__$foo(uint8_t arg1);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a freestanding function that returns a value.
    #[test]
    fn freestanding_function_with_return() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    fn foo() -> u8;
                }
            }
        };
        let expected = r#"
#include <stdint.h>
uint8_t __swift_bridge__$foo(void);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we add a `typedef struct` for types in the extern "Rust" block.
    #[test]
    fn type_definition() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    type SomeType;
                }
            }
        };
        let expected = r#"
typedef struct SomeType SomeType;
void __swift_bridge__$SomeType$_free(void* self);
"#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a method with no arguments.
    #[test]
    fn method_no_args() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    type SomeType;
                    fn a(self);
                    fn b(&self);
                    fn c(&mut self);
                    fn d(self: SomeType);
                    fn e(self: &SomeType);
                    fn f(self: &mut SomeType);
                }
            }
        };
        let expected = r#"
typedef struct SomeType SomeType;
void __swift_bridge__$SomeType$_free(void* self);
void __swift_bridge__$SomeType$a(void* self);
void __swift_bridge__$SomeType$b(void* self);
void __swift_bridge__$SomeType$c(void* self);
void __swift_bridge__$SomeType$d(void* self);
void __swift_bridge__$SomeType$e(void* self);
void __swift_bridge__$SomeType$f(void* self);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a method with no arguments.
    #[test]
    fn method_one_arg() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    type SomeType;
                    fn foo(&self, val: u8);
                }
            }
        };
        let expected = r#"
#include <stdint.h>
typedef struct SomeType SomeType;
void __swift_bridge__$SomeType$_free(void* self);
void __swift_bridge__$SomeType$foo(void* self, uint8_t val);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a method with an opaque argument.
    #[test]
    fn method_one_opaque_arg() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    type SomeType;
                    fn foo(&self, val: SomeType);
                }
            }
        };
        let expected = r#"
typedef struct SomeType SomeType;
void __swift_bridge__$SomeType$_free(void* self);
void __swift_bridge__$SomeType$foo(void* self, void* val);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we generate a type definition for a method that has a return type.
    #[test]
    fn method_with_return() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    type SomeType;
                    fn foo(&self) -> u8;
                }
            }
        };
        let expected = r#"
#include <stdint.h>
typedef struct SomeType SomeType;
void __swift_bridge__$SomeType$_free(void* self);
uint8_t __swift_bridge__$SomeType$foo(void* self);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    /// Verify that we define a FfiSlice_T struct if we return a slice of type T.
    /// We make sure to only define one instance of FfiSlice_T even if there are multiple functions
    /// that need it.
    #[test]
    fn slice_return() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                extern "Rust" {
                    fn foo() -> &'static [u8];
                    fn bar() -> &'static [u8];
                }
            }
        };
        let expected = r#"
#include <stdint.h>
typedef struct FfiSlice_uint8_t { uint8_t* start; uintptr_t len; } FfiSlice_uint8_t;
struct __private__FfiSlice __swift_bridge__$foo(void);
struct __private__FfiSlice __swift_bridge__$bar(void);
        "#;

        let module = parse_ok(tokens);
        assert_eq!(module.generate_c_header_inner().trim(), expected.trim());
    }

    fn parse_ok(tokens: TokenStream) -> SwiftBridgeModule {
        let module_and_errors: SwiftBridgeModuleAndErrors = syn::parse2(tokens).unwrap();
        module_and_errors.module
    }

    /// Verify that we emit a typedef for a struct with no fields.
    #[test]
    fn struct_with_no_fields() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                struct Foo;
                struct Bar{}
                struct Bazz();
            }
        };
        let expected = r#"
typedef struct Foo Foo;
typedef struct Bar Bar;
typedef struct Bazz Bazz;
        "#;

        let module = parse_ok(tokens);
        assert_generated_equals_expected(&module.generate_c_header_inner(), &expected);
    }

    /// Verify that we emit a typedef for a struct with one fields.
    #[test]
    fn struct_with_one_field() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                #[swift_bridge(swift_repr = "struct")]
                struct Foo {
                    field: u8
                }
                struct Bar(u8);
            }
        };
        let expected = r#"
#include <stdint.h>
typedef struct Foo { uint8_t field; } Foo;
typedef struct Bar { uint8_t _0; } Bar;
        "#;

        let module = parse_ok(tokens);
        assert_generated_equals_expected(&module.generate_c_header_inner(), &expected);
    }

    /// Verify that we emit a typedef for a struct with two field.
    #[test]
    fn struct_with_two_fields() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                #[swift_bridge(swift_repr = "struct")]
                struct Foo {
                    field1: u8,
                    field2: u16
                }
            }
        };
        let expected = r#"
#include <stdint.h>
typedef struct Foo { uint8_t field1; uint16_t field2; } Foo;
        "#;

        let module = parse_ok(tokens);
        assert_generated_equals_expected(&module.generate_c_header_inner(), &expected);
    }

    /// Verify that we use the swift_name when generating the struct typedef.
    #[test]
    fn uses_swift_name_struct_attribute() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                #[swift_bridge(swift_name = "FfiFoo")]
                struct Foo;
            }
        };
        let expected = r#"
typedef struct FfiFoo FfiFoo;
        "#;

        let module = parse_ok(tokens);
        assert_generated_equals_expected(&module.generate_c_header_inner(), &expected);
    }

    /// Verify that we use the struct's swift_name attribute when generating function signatures.
    #[test]
    fn uses_swift_name_for_function_args_and_returns() {
        let tokens = quote! {
            #[swift_bridge::bridge]
            mod ffi {
                #[swift_bridge(swift_name = "FfiFoo")]
                struct Foo;

                extern "Rust" {
                    fn some_function(arg: Foo) -> Foo;
                }
            }
        };
        let expected = r#"
typedef struct FfiFoo FfiFoo;
struct FfiFoo __swift_bridge__$some_function(struct FfiFoo arg);
        "#;

        let module = parse_ok(tokens);
        assert_generated_equals_expected(&module.generate_c_header_inner(), &expected);
    }
}
