use std::collections::HashMap;

use inkwell::builder::BuilderError;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{FunctionType, IntType, PointerType, StructType};
use inkwell::values::{FunctionValue, IntValue, StructValue};
use inkwell::AddressSpace;

use crate::transform::{AnfAtom, AnfExpr, AnfFunction, AnfProgram, AnfRhs};

pub fn generate_llvm_ir(program: &AnfProgram) -> Result<String, String> {
    let context = Context::create();
    let module = context.create_module("lamc");
    let builder = context.create_builder();

    let mut cg = Codegen::new(&context, module, builder)?;
    cg.emit_program(program)?;

    Ok(cg.module.print_to_string().to_string())
}

struct Codegen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: inkwell::builder::Builder<'ctx>,
    i64_type: IntType<'ctx>,
    i8_ptr_type: PointerType<'ctx>,
    value_ty: StructType<'ctx>,
    value_ptr_type: PointerType<'ctx>,
    closure_ty: StructType<'ctx>,
    closure_ptr_type: PointerType<'ctx>,
    prim2_ty: StructType<'ctx>,
    prim2_ptr_type: PointerType<'ctx>,
    call_fn_type: FunctionType<'ctx>,
    call_fn_ptr_type: PointerType<'ctx>,
    malloc_fn: FunctionValue<'ctx>,
    print_int_fn: FunctionValue<'ctx>,
}

impl<'ctx> Codegen<'ctx> {
    fn new(
        context: &'ctx Context,
        module: Module<'ctx>,
        builder: inkwell::builder::Builder<'ctx>,
    ) -> Result<Self, String> {
        let i64_type = context.i64_type();
        let i8_ptr_type = context.ptr_type(AddressSpace::default());

        let value_ty = context.opaque_struct_type("Value");
        value_ty.set_body(&[i64_type.into(), i64_type.into()], false);

        let closure_ty = context.opaque_struct_type("Closure");
        closure_ty.set_body(&[i8_ptr_type.into(), i8_ptr_type.into()], false);

        let value_ptr_type = context.ptr_type(AddressSpace::default());
        let closure_ptr_type = context.ptr_type(AddressSpace::default());

        let prim2_ty = context.opaque_struct_type("Prim2");
        prim2_ty.set_body(
            &[
                i64_type.into(),
                i64_type.into(),
                value_ty.into(),
                value_ty.into(),
            ],
            false,
        );
        let prim2_ptr_type = context.ptr_type(AddressSpace::default());

        let call_fn_type = value_ty.fn_type(&[i8_ptr_type.into(), value_ty.into()], false);
        let call_fn_ptr_type = context.ptr_type(AddressSpace::default());

        let malloc_type = i8_ptr_type.fn_type(&[i64_type.into()], false);
        let malloc_fn = module.add_function("malloc", malloc_type, None);

        let print_type = context.void_type().fn_type(&[i64_type.into()], false);
        let print_int_fn = module.add_function("print_int", print_type, None);

        Ok(Self {
            context,
            module,
            builder,
            i64_type,
            i8_ptr_type,
            value_ty,
            value_ptr_type,
            closure_ty,
            closure_ptr_type,
            prim2_ty,
            prim2_ptr_type,
            call_fn_type,
            call_fn_ptr_type,
            malloc_fn,
            print_int_fn,
        })
    }

    fn emit_program(&mut self, program: &AnfProgram) -> Result<(), String> {
        let mut fn_map = HashMap::new();
        for f in &program.functions {
            let func = self.module.add_function(&f.name, self.call_fn_type, None);
            fn_map.insert(f.name.clone(), func);
        }

        for f in &program.functions {
            let func = *fn_map
                .get(&f.name)
                .ok_or_else(|| format!("missing function declaration: {}", f.name))?;
            self.emit_lifted_function(f, func, &fn_map)?;
        }

        self.emit_main(&program.main, &fn_map)
    }

    fn emit_lifted_function(
        &mut self,
        func: &AnfFunction,
        llvm_func: FunctionValue<'ctx>,
        fn_map: &HashMap<String, FunctionValue<'ctx>>,
    ) -> Result<(), String> {
        let entry = self.context.append_basic_block(llvm_func, "entry");
        self.builder.position_at_end(entry);

        let env_ptr = llvm_func
            .get_nth_param(0)
            .ok_or_else(|| "missing env parameter".to_string())?
            .into_pointer_value();
        let arg = llvm_func
            .get_nth_param(1)
            .ok_or_else(|| "missing arg parameter".to_string())?
            .into_struct_value();

        let mut vars = HashMap::new();
        vars.insert(func.param.clone(), arg);

        if !func.free_vars.is_empty() {
            let env_values_ptr = self
                .builder
                .build_pointer_cast(env_ptr, self.value_ptr_type, "env_values")
                .map_err(builder_err)?;

            for (idx, name) in func.free_vars.iter().enumerate() {
                let index = self.i64_type.const_int(idx as u64, false);
                let slot = unsafe {
                    self.builder
                        .build_gep(self.value_ty, env_values_ptr, &[index], "env_slot")
                }
                .map_err(builder_err)?;

                let loaded = self
                    .builder
                    .build_load(self.value_ty, slot, "env_load")
                    .map_err(builder_err)?
                    .into_struct_value();
                vars.insert(name.clone(), loaded);
            }
        }

        let body = self.emit_anf_expr(&func.body, llvm_func, fn_map, &mut vars)?;
        self.builder
            .build_return(Some(&body))
            .map_err(builder_err)?;

        Ok(())
    }

    fn emit_main(
        &mut self,
        main_expr: &AnfExpr,
        fn_map: &HashMap<String, FunctionValue<'ctx>>,
    ) -> Result<(), String> {
        let main_type = self.i64_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        let mut vars = HashMap::new();
        let result = self.emit_anf_expr(main_expr, main_fn, fn_map, &mut vars)?;

        let tag = self.extract_tag(result)?;
        let payload = self.extract_payload(result)?;
        let is_int = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                tag,
                self.i64_type.const_zero(),
                "is_int",
            )
            .map_err(builder_err)?;
        let ret = self
            .builder
            .build_select(is_int, payload, self.i64_type.const_zero(), "ret")
            .map_err(builder_err)?
            .into_int_value();

        self.builder.build_return(Some(&ret)).map_err(builder_err)?;

        Ok(())
    }

    fn emit_anf_expr(
        &mut self,
        expr: &AnfExpr,
        current_fn: FunctionValue<'ctx>,
        fn_map: &HashMap<String, FunctionValue<'ctx>>,
        vars: &mut HashMap<String, StructValue<'ctx>>,
    ) -> Result<StructValue<'ctx>, String> {
        match expr {
            AnfExpr::Let(name, rhs, rest) => {
                let value = self.emit_rhs(rhs, current_fn, fn_map, vars)?;
                vars.insert(name.clone(), value);
                self.emit_anf_expr(rest, current_fn, fn_map, vars)
            }
            AnfExpr::Return(atom) => self.emit_atom(atom, fn_map, vars),
        }
    }

    fn emit_rhs(
        &mut self,
        rhs: &AnfRhs,
        current_fn: FunctionValue<'ctx>,
        fn_map: &HashMap<String, FunctionValue<'ctx>>,
        vars: &HashMap<String, StructValue<'ctx>>,
    ) -> Result<StructValue<'ctx>, String> {
        match rhs {
            AnfRhs::App(f, a) => {
                let callee = self.emit_atom(f, fn_map, vars)?;
                let arg = self.emit_atom(a, fn_map, vars)?;
                self.emit_apply(current_fn, callee, arg)
            }
        }
    }

    fn emit_atom(
        &mut self,
        atom: &AnfAtom,
        fn_map: &HashMap<String, FunctionValue<'ctx>>,
        vars: &HashMap<String, StructValue<'ctx>>,
    ) -> Result<StructValue<'ctx>, String> {
        match atom {
            AnfAtom::Var(v) => vars
                .get(v)
                .copied()
                .ok_or_else(|| format!("unbound variable in codegen: {v}")),
            AnfAtom::Int(n) => self.make_int(*n),
            AnfAtom::Prim(name) => match name.as_str() {
                "true" => self.make_int(1),
                "false" => self.make_int(0),
                "print" => self.make_prim(0),
                "add" => self.make_prim(1),
                "sub" => self.make_prim(2),
                "mul" => self.make_prim(3),
                "if" => self.make_prim(4),
                _ => Err(format!("unknown primitive: {name}")),
            },
            AnfAtom::MakeClosure { func, captures } => {
                let target = *fn_map
                    .get(func)
                    .ok_or_else(|| format!("unknown function in closure: {func}"))?;
                self.make_closure(target, captures, vars)
            }
        }
    }

    fn emit_apply(
        &mut self,
        current_fn: FunctionValue<'ctx>,
        callee: StructValue<'ctx>,
        arg: StructValue<'ctx>,
    ) -> Result<StructValue<'ctx>, String> {
        let tag = self.extract_tag(callee)?;
        let payload = self.extract_payload(callee)?;

        let closure_block = self.context.append_basic_block(current_fn, "app_closure");
        let prim1_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_check");
        let prim1_dispatch_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_dispatch");
        let prim1_print_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_print");
        let prim1_add_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_add_check");
        let prim1_sub_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_sub_check");
        let prim1_mul_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_mul_check");
        let prim1_if_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_if_check");
        let prim1_partial_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_partial");
        let prim1_unknown_block = self
            .context
            .append_basic_block(current_fn, "app_prim1_unknown");

        let prim2_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_check");
        let prim2_dispatch_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_dispatch");
        let prim2_add_stage_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_add_stage_check");
        let prim2_add_block = self.context.append_basic_block(current_fn, "app_prim2_add");
        let prim2_sub_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_sub_check");
        let prim2_sub_stage_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_sub_stage_check");
        let prim2_sub_block = self.context.append_basic_block(current_fn, "app_prim2_sub");
        let prim2_mul_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_mul_check");
        let prim2_mul_stage_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_mul_stage_check");
        let prim2_mul_block = self.context.append_basic_block(current_fn, "app_prim2_mul");
        let prim2_if_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_if_check");
        let prim2_if_stage1_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_if_stage1_check");
        let prim2_if_stage1_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_if_stage1");
        let prim2_if_stage2_check_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_if_stage2_check");
        let prim2_if_stage2_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_if_stage2");
        let prim2_unknown_block = self
            .context
            .append_basic_block(current_fn, "app_prim2_unknown");

        let bad_block = self.context.append_basic_block(current_fn, "app_bad");
        let merge_block = self.context.append_basic_block(current_fn, "app_merge");

        let is_closure = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                tag,
                self.i64_type.const_int(1, false),
                "is_closure",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_closure, closure_block, prim1_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(closure_block);
        let closure_raw = self
            .builder
            .build_int_to_ptr(payload, self.i8_ptr_type, "closure_raw")
            .map_err(builder_err)?;
        let closure_ptr = self
            .builder
            .build_pointer_cast(closure_raw, self.closure_ptr_type, "closure_ptr")
            .map_err(builder_err)?;
        let fn_slot = self
            .builder
            .build_struct_gep(self.closure_ty, closure_ptr, 0, "fn_slot")
            .map_err(builder_err)?;
        let env_slot = self
            .builder
            .build_struct_gep(self.closure_ty, closure_ptr, 1, "env_slot")
            .map_err(builder_err)?;
        let fn_i8 = self
            .builder
            .build_load(self.i8_ptr_type, fn_slot, "fn_i8")
            .map_err(builder_err)?
            .into_pointer_value();
        let env_i8 = self
            .builder
            .build_load(self.i8_ptr_type, env_slot, "env_i8")
            .map_err(builder_err)?
            .into_pointer_value();
        let typed_fn = self
            .builder
            .build_pointer_cast(fn_i8, self.call_fn_ptr_type, "typed_fn")
            .map_err(builder_err)?;
        let call = self
            .builder
            .build_indirect_call(
                self.call_fn_type,
                typed_fn,
                &[env_i8.into(), arg.into()],
                "closure_call",
            )
            .map_err(builder_err)?;
        let closure_result = call.try_as_basic_value().unwrap_basic().into_struct_value();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let closure_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing closure block".to_string())?;

        self.builder.position_at_end(prim1_check_block);
        let is_prim1 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                tag,
                self.i64_type.const_int(2, false),
                "is_prim1",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_prim1, prim1_dispatch_block, prim2_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_dispatch_block);
        let is_print = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                payload,
                self.i64_type.const_zero(),
                "is_print",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_print, prim1_print_block, prim1_add_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_print_block);
        let arg_payload = self.extract_payload(arg)?;
        self.builder
            .build_call(self.print_int_fn, &[arg_payload.into()], "print_call")
            .map_err(builder_err)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim1_print_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim1 print block".to_string())?;

        self.builder.position_at_end(prim1_add_check_block);
        let is_add = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                payload,
                self.i64_type.const_int(1, false),
                "is_add",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_add, prim1_partial_block, prim1_sub_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_sub_check_block);
        let is_sub = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                payload,
                self.i64_type.const_int(2, false),
                "is_sub",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_sub, prim1_partial_block, prim1_mul_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_mul_check_block);
        let is_mul = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                payload,
                self.i64_type.const_int(3, false),
                "is_mul",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_mul, prim1_partial_block, prim1_if_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_if_check_block);
        let is_if = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                payload,
                self.i64_type.const_int(4, false),
                "is_if",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_if, prim1_partial_block, prim1_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim1_partial_block);
        let prim1_partial_result = self.make_prim_partial(
            payload,
            self.i64_type.const_int(1, false),
            arg,
            self.value_ty.get_undef(),
        )?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim1_partial_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim1 partial block".to_string())?;

        self.builder.position_at_end(prim1_unknown_block);
        let prim1_unknown_result = self.make_int(0)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim1_unknown_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim1 unknown block".to_string())?;

        self.builder.position_at_end(prim2_check_block);
        let is_prim2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                tag,
                self.i64_type.const_int(3, false),
                "is_prim2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_prim2, prim2_dispatch_block, bad_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_dispatch_block);
        let prim2_raw = self
            .builder
            .build_int_to_ptr(payload, self.i8_ptr_type, "prim2_raw")
            .map_err(builder_err)?;
        let prim2_ptr = self
            .builder
            .build_pointer_cast(prim2_raw, self.prim2_ptr_type, "prim2_ptr")
            .map_err(builder_err)?;
        let op_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 0, "prim2_op_slot")
            .map_err(builder_err)?;
        let stage_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 1, "prim2_stage_slot")
            .map_err(builder_err)?;
        let v0_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 2, "prim2_v0_slot")
            .map_err(builder_err)?;
        let v1_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 3, "prim2_v1_slot")
            .map_err(builder_err)?;

        let op_id = self
            .builder
            .build_load(self.i64_type, op_slot, "prim2_op")
            .map_err(builder_err)?
            .into_int_value();
        let stage = self
            .builder
            .build_load(self.i64_type, stage_slot, "prim2_stage")
            .map_err(builder_err)?
            .into_int_value();
        let v0 = self
            .builder
            .build_load(self.value_ty, v0_slot, "prim2_v0")
            .map_err(builder_err)?
            .into_struct_value();
        let v1 = self
            .builder
            .build_load(self.value_ty, v1_slot, "prim2_v1")
            .map_err(builder_err)?
            .into_struct_value();

        let is_add2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                op_id,
                self.i64_type.const_int(1, false),
                "is_add2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_add2, prim2_add_stage_check_block, prim2_sub_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_add_stage_check_block);
        let add_stage_ok = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                stage,
                self.i64_type.const_int(1, false),
                "add_stage_ok",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(add_stage_ok, prim2_add_block, prim2_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_sub_check_block);
        let is_sub2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                op_id,
                self.i64_type.const_int(2, false),
                "is_sub2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_sub2, prim2_sub_stage_check_block, prim2_mul_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_sub_stage_check_block);
        let sub_stage_ok = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                stage,
                self.i64_type.const_int(1, false),
                "sub_stage_ok",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(sub_stage_ok, prim2_sub_block, prim2_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_mul_check_block);
        let is_mul2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                op_id,
                self.i64_type.const_int(3, false),
                "is_mul2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_mul2, prim2_mul_stage_check_block, prim2_if_check_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_mul_stage_check_block);
        let mul_stage_ok = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                stage,
                self.i64_type.const_int(1, false),
                "mul_stage_ok",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(mul_stage_ok, prim2_mul_block, prim2_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_if_check_block);
        let is_if2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                op_id,
                self.i64_type.const_int(4, false),
                "is_if2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(is_if2, prim2_if_stage1_check_block, prim2_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_if_stage1_check_block);
        let if_stage1 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                stage,
                self.i64_type.const_int(1, false),
                "if_stage1",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(
                if_stage1,
                prim2_if_stage1_block,
                prim2_if_stage2_check_block,
            )
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_if_stage2_check_block);
        let if_stage2 = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                stage,
                self.i64_type.const_int(2, false),
                "if_stage2",
            )
            .map_err(builder_err)?;
        self.builder
            .build_conditional_branch(if_stage2, prim2_if_stage2_block, prim2_unknown_block)
            .map_err(builder_err)?;

        self.builder.position_at_end(prim2_add_block);
        let lhs_add = self.extract_payload(v0)?;
        let rhs_add = self.extract_payload(arg)?;
        let add_out = self
            .builder
            .build_int_add(lhs_add, rhs_add, "add_out")
            .map_err(builder_err)?;
        let prim2_add_result = self.make_int_from_value(add_out)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_add_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 add block".to_string())?;

        self.builder.position_at_end(prim2_sub_block);
        let lhs_sub = self.extract_payload(v0)?;
        let rhs_sub = self.extract_payload(arg)?;
        let sub_out = self
            .builder
            .build_int_sub(lhs_sub, rhs_sub, "sub_out")
            .map_err(builder_err)?;
        let prim2_sub_result = self.make_int_from_value(sub_out)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_sub_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 sub block".to_string())?;

        self.builder.position_at_end(prim2_mul_block);
        let lhs_mul = self.extract_payload(v0)?;
        let rhs_mul = self.extract_payload(arg)?;
        let mul_out = self
            .builder
            .build_int_mul(lhs_mul, rhs_mul, "mul_out")
            .map_err(builder_err)?;
        let prim2_mul_result = self.make_int_from_value(mul_out)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_mul_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 mul block".to_string())?;

        self.builder.position_at_end(prim2_if_stage1_block);
        let prim2_if_stage1_result =
            self.make_prim_partial(op_id, self.i64_type.const_int(2, false), v0, arg)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_if_stage1_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 if stage1 block".to_string())?;

        self.builder.position_at_end(prim2_if_stage2_block);
        let cond_payload = self.extract_payload(v0)?;
        let cond_true = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                cond_payload,
                self.i64_type.const_zero(),
                "cond_true",
            )
            .map_err(builder_err)?;
        let prim2_if_stage2_result = self
            .builder
            .build_select(cond_true, v1, arg, "if_out")
            .map_err(builder_err)?
            .into_struct_value();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_if_stage2_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 if stage2 block".to_string())?;

        self.builder.position_at_end(prim2_unknown_block);
        let prim2_unknown_result = self.make_int(0)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let prim2_unknown_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing prim2 unknown block".to_string())?;

        self.builder.position_at_end(bad_block);
        let bad_result = self.make_int(0)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(builder_err)?;
        let bad_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "missing bad block".to_string())?;

        self.builder.position_at_end(merge_block);
        let phi = self
            .builder
            .build_phi(self.value_ty, "app_result")
            .map_err(builder_err)?;
        phi.add_incoming(&[
            (&closure_result, closure_end),
            (&arg, prim1_print_end),
            (&prim1_partial_result, prim1_partial_end),
            (&prim1_unknown_result, prim1_unknown_end),
            (&prim2_add_result, prim2_add_end),
            (&prim2_sub_result, prim2_sub_end),
            (&prim2_mul_result, prim2_mul_end),
            (&prim2_if_stage1_result, prim2_if_stage1_end),
            (&prim2_if_stage2_result, prim2_if_stage2_end),
            (&prim2_unknown_result, prim2_unknown_end),
            (&bad_result, bad_end),
        ]);

        Ok(phi.as_basic_value().into_struct_value())
    }

    fn make_closure(
        &mut self,
        target: FunctionValue<'ctx>,
        captures: &[String],
        vars: &HashMap<String, StructValue<'ctx>>,
    ) -> Result<StructValue<'ctx>, String> {
        let bytes_per_value = 16u64;
        let env_bytes = (captures.len() as u64)
            .saturating_mul(bytes_per_value)
            .max(1);
        let env_raw = self
            .builder
            .build_call(
                self.malloc_fn,
                &[self.i64_type.const_int(env_bytes, false).into()],
                "env_alloc",
            )
            .map_err(builder_err)?
            .try_as_basic_value()
            .unwrap_basic()
            .into_pointer_value();

        let env_values_ptr = self
            .builder
            .build_pointer_cast(env_raw, self.value_ptr_type, "env_values_ptr")
            .map_err(builder_err)?;

        for (idx, name) in captures.iter().enumerate() {
            let captured = vars
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown captured variable: {name}"))?;
            let index = self.i64_type.const_int(idx as u64, false);
            let slot = unsafe {
                self.builder
                    .build_gep(self.value_ty, env_values_ptr, &[index], "capture_slot")
            }
            .map_err(builder_err)?;
            self.builder
                .build_store(slot, captured)
                .map_err(builder_err)?;
        }

        let closure_raw = self
            .builder
            .build_call(
                self.malloc_fn,
                &[self.i64_type.const_int(16, false).into()],
                "closure_alloc",
            )
            .map_err(builder_err)?
            .try_as_basic_value()
            .unwrap_basic()
            .into_pointer_value();

        let closure_ptr = self
            .builder
            .build_pointer_cast(closure_raw, self.closure_ptr_type, "closure_ptr")
            .map_err(builder_err)?;

        let fn_ptr = target.as_global_value().as_pointer_value();
        let fn_i8 = self
            .builder
            .build_pointer_cast(fn_ptr, self.i8_ptr_type, "fn_i8")
            .map_err(builder_err)?;

        let fn_slot = self
            .builder
            .build_struct_gep(self.closure_ty, closure_ptr, 0, "closure_fn_slot")
            .map_err(builder_err)?;
        let env_slot = self
            .builder
            .build_struct_gep(self.closure_ty, closure_ptr, 1, "closure_env_slot")
            .map_err(builder_err)?;

        self.builder
            .build_store(fn_slot, fn_i8)
            .map_err(builder_err)?;
        self.builder
            .build_store(env_slot, env_raw)
            .map_err(builder_err)?;

        let closure_as_int = self
            .builder
            .build_ptr_to_int(closure_raw, self.i64_type, "closure_as_int")
            .map_err(builder_err)?;

        self.make_value(self.i64_type.const_int(1, false), closure_as_int)
    }

    fn make_int(&self, n: i64) -> Result<StructValue<'ctx>, String> {
        self.make_value(
            self.i64_type.const_zero(),
            self.i64_type.const_int(n as u64, true),
        )
    }

    fn make_int_from_value(&self, v: IntValue<'ctx>) -> Result<StructValue<'ctx>, String> {
        self.make_value(self.i64_type.const_zero(), v)
    }

    fn make_prim(&self, id: u64) -> Result<StructValue<'ctx>, String> {
        self.make_value(
            self.i64_type.const_int(2, false),
            self.i64_type.const_int(id, false),
        )
    }

    fn make_prim_partial(
        &mut self,
        prim_id: IntValue<'ctx>,
        stage: IntValue<'ctx>,
        v0: StructValue<'ctx>,
        v1: StructValue<'ctx>,
    ) -> Result<StructValue<'ctx>, String> {
        let prim2_raw = self
            .builder
            .build_call(
                self.malloc_fn,
                &[self.i64_type.const_int(48, false).into()],
                "prim2_alloc",
            )
            .map_err(builder_err)?
            .try_as_basic_value()
            .unwrap_basic()
            .into_pointer_value();

        let prim2_ptr = self
            .builder
            .build_pointer_cast(prim2_raw, self.prim2_ptr_type, "prim2_ptr")
            .map_err(builder_err)?;

        let op_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 0, "prim2_op_slot")
            .map_err(builder_err)?;
        let stage_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 1, "prim2_stage_slot")
            .map_err(builder_err)?;
        let v0_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 2, "prim2_v0_slot")
            .map_err(builder_err)?;
        let v1_slot = self
            .builder
            .build_struct_gep(self.prim2_ty, prim2_ptr, 3, "prim2_v1_slot")
            .map_err(builder_err)?;

        self.builder
            .build_store(op_slot, prim_id)
            .map_err(builder_err)?;
        self.builder
            .build_store(stage_slot, stage)
            .map_err(builder_err)?;
        self.builder.build_store(v0_slot, v0).map_err(builder_err)?;
        self.builder.build_store(v1_slot, v1).map_err(builder_err)?;

        let raw_i64 = self
            .builder
            .build_ptr_to_int(prim2_raw, self.i64_type, "prim2_ptr_i64")
            .map_err(builder_err)?;

        self.make_value(self.i64_type.const_int(3, false), raw_i64)
    }

    fn make_value(
        &self,
        tag: IntValue<'ctx>,
        payload: IntValue<'ctx>,
    ) -> Result<StructValue<'ctx>, String> {
        let v0 = self.value_ty.get_undef();
        let v1 = self
            .builder
            .build_insert_value(v0, tag, 0, "v_tag")
            .map_err(builder_err)?
            .into_struct_value();
        let v2 = self
            .builder
            .build_insert_value(v1, payload, 1, "v_payload")
            .map_err(builder_err)?
            .into_struct_value();
        Ok(v2)
    }

    fn extract_tag(&self, value: StructValue<'ctx>) -> Result<IntValue<'ctx>, String> {
        Ok(self
            .builder
            .build_extract_value(value, 0, "tag")
            .map_err(builder_err)?
            .into_int_value())
    }

    fn extract_payload(&self, value: StructValue<'ctx>) -> Result<IntValue<'ctx>, String> {
        Ok(self
            .builder
            .build_extract_value(value, 1, "payload")
            .map_err(builder_err)?
            .into_int_value())
    }
}

fn builder_err(err: BuilderError) -> String {
    err.to_string()
}
