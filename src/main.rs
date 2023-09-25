use std::collections::VecDeque;
use std::{default, fs};
use std::path::Path;
use std::process::Command;

use inkwell::basic_block::BasicBlock;
use inkwell::{AddressSpace, OptimizationLevel};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{Target, InitializationConfig, TargetMachine, RelocMode, CodeModel, FileType};
use inkwell::types::{VoidType, FunctionType};
use inkwell::values::{BasicValueEnum, PointerValue};


#[derive(Clone, Debug)]
enum Op {
    PointerInc(usize),
    PointerDec(usize),
    ValueInc(usize),
    ValueDec(usize),
    Output,
    Input,
    LLoop,
    RLoop,
}


struct Lexer {
    buffer: Vec<char>,
    ptr: usize
}

impl Lexer {
    fn new(source: Vec<char>) -> Self {
        Self {
            buffer: source,
            ptr: 0
        }
    }

    fn peek(&self) -> Option<&char> {
        return self.buffer.get(self.ptr);
    }

    fn eat(&mut self) -> Option<&char> {
        let ptr = self.ptr;
        self.ptr += 1;
        return self.buffer.get(ptr);
    }

    fn eat_while_same(&mut self, c: &char) -> Op {
        let mut count = 0;
        while let Some(ch) = self.peek() {
            if ch == c {
                self.eat();
                count += 1;
            } else {
                break;
            }
        }
        match c {
            '>' => Op::PointerInc(count),
            '<' => Op::PointerDec(count),
            '+' => Op::ValueInc(count),
            '-' => Op::ValueDec(count),
            t => unreachable!("Illegal character {}", t)
        }
    }


    fn run(&mut self) -> Vec<Op> {
        let mut vec = Vec::new();
        loop {
            let c = if let Some(c) = self.peek() {
                c.clone()
            } else {
                break;
            };
            match c {
                '>' | '<' | '+' | '-' => {
                    let op = self.eat_while_same(&c);
                    vec.push(op);
                },
                '.' => {
                    self.eat().unwrap();
                    vec.push(Op::Output);
                }
                ',' => {
                    self.eat().unwrap();
                    vec.push(Op::Input);
                }
                '[' => {
                    self.eat().unwrap();
                    vec.push(Op::LLoop);
                }
                ']' => {
                    self.eat().unwrap();
                    vec.push(Op::RLoop);
                }
                '\n' | '\r' | ' ' | '\t' => {
                    self.eat().unwrap();
                }
                _ => unimplemented!()
            }
        }
        vec
    }
}

struct CodeGen<'a> {
    ctx: &'a Context,
    builder: Builder<'a>,
    ptr: PointerValue<'a>,
    module: Module<'a>,
    loops: VecDeque<(BasicBlock<'a>, BasicBlock<'a>)>,
    ast: Vec<Op>,
}

impl<'a> CodeGen<'a> {

    fn ptr_manipulate(&mut self, count: usize, dec: bool) {
        let v = self.builder.build_load(self.ptr.into(), "load_ptr").unwrap();
        let mut int_val = self.ctx.i64_type().const_int(count as u64, false);
        if dec {
            int_val = int_val.const_neg();
        }
        let ptr = unsafe {
            let inc = self.builder.build_gep(v.into_pointer_value(), &[int_val.into()], "gep");
            inc.unwrap()
        };
        let _ = self.builder.build_store(self.ptr, ptr);
    }

    fn val_manipulate(&mut self, count: usize, dec: bool) {
        let v = self.builder.build_load(self.ptr.into(), "load_ptr").unwrap();
        let val = self.builder.build_load(v.into_pointer_value(), "load_val").unwrap();
        let int_val = self.ctx.i64_type().const_int(count as u64, false);
        let new_val = if !dec {
            let val = self.builder.build_int_add( val.into_int_value(), int_val, "add").unwrap();
            val
        } else {

            let val = self.builder.build_int_sub(val.into_int_value(), int_val, "sub").unwrap();
            val
        };
        let _ = self.builder.build_store(v.into_pointer_value(), new_val).unwrap();
    }

    fn out(&mut self) {
        let v = self.builder.build_load(self.ptr.into(), "load_ptr").unwrap();
        let val = self.builder.build_load(v.into_pointer_value(), "load_val").unwrap();
        let putchar = self.module.get_function("putchar").unwrap();
        let _call = self.builder.build_call(putchar, &[val.into()], "out").unwrap();
    }

    fn input(&mut self) {
        let v = self.builder.build_load(self.ptr.into(), "load_ptr").unwrap();
        let putchar = self.module.get_function("getchar").unwrap();
        let call = self.builder.build_call(putchar, &[], "in").unwrap().try_as_basic_value().left().unwrap();
        let _ = self.builder.build_store(v.into_pointer_value(), call);
    }

    fn loop_start(&mut self) {
        let start_block = self.builder.get_insert_block().unwrap();
        let main = start_block.get_parent().unwrap();
        let cond_block = self.ctx.append_basic_block(main, "cond_block");
        let body_block = self.ctx.append_basic_block(main, "body_block");
        let end_block = self.ctx.append_basic_block(main, "end_block");
        self.loops.push_back((cond_block, end_block));
        self.builder.build_unconditional_branch(cond_block);
        self.builder.position_at_end(cond_block);
        let v = self.builder.build_load(self.ptr.into(), "load_ptr").unwrap();
        let val = self.builder.build_load(v.into_pointer_value(), "load_val").unwrap();
        let comp = self.builder.build_int_compare(inkwell::IntPredicate::NE, val.into_int_value(), self.ctx.i8_type().const_zero(), "ne_zero")
.unwrap();
        self.builder.build_conditional_branch(comp, body_block, end_block);
        self.builder.position_at_end(body_block);
    }

    fn loop_end(&mut self) {
        let (cond_block, end_block) = self.loops.pop_back().unwrap();
        self.builder.build_unconditional_branch(cond_block);
        self.builder.position_at_end(end_block);
    }

    fn new(ctx: &'a Context, ast: Vec<Op>) -> Self {
        let builder = ctx.create_builder();
        let module = ctx.create_module("main");
        let i8_type = ctx.i8_type();
        let i32_type = ctx.i32_type();
        let i8_ptr = i8_type.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();
        let _putchar = module.add_function("putchar", i8_type.fn_type(&[i32_type.into()], false), None);
        let _getchar = module.add_function("getchar", i8_type.fn_type(&[], false), None);
        let calloc = module.add_function("calloc", i8_ptr.fn_type(&[i64_type.into(), i64_type.into()], false), None);
        let fn_type = i8_type.fn_type(&[], false);
        let func = module.add_function("main", fn_type, None);
        let block = ctx.append_basic_block(func, "entry");
        builder.position_at_end(block);

        let ptr_val = builder.build_alloca(i8_ptr, "ptr").unwrap();


        let args = (i64_type.const_int(1000, false), i64_type.const_int(1, false)); 
        let calloc_block = builder.build_call(calloc, &[args.0.into(), args.1.into()], "block").unwrap().try_as_basic_value().left();
        let _i = builder.build_store(ptr_val, calloc_block.unwrap()).unwrap();
        Self {
            ctx: &ctx,
            builder,
            ptr: ptr_val,
            module,
            loops: VecDeque::new(),
            ast
        }
    }

    pub fn run(&mut self) {
        for op in self.ast.clone().drain(..) {
            match op {
                Op::PointerInc(v) => {
                    self.ptr_manipulate(v, false);
                }
                Op::PointerDec(v) => {
                    self.ptr_manipulate(v, true);
                }
                Op::ValueInc(v) => {
                    self.val_manipulate(v, false);
                }
                Op::ValueDec(v) => {
                    self.val_manipulate(v, true);
                }
                Op::Output => {
                    self.out();
                }
                Op::Input => {
                    self.input();
                }
                Op::LLoop => {
                    self.loop_start();
                }
                Op::RLoop => {
                    self.loop_end();
                }
            }
        }
        let _ret = self.builder.build_return(Some(&self.ctx.i8_type().const_int(0, false)));
    }

    pub fn generate_machine_code(&self, path: &str) {
        Target::initialize_all(&InitializationConfig::default());
        let target_triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&target_triple).unwrap();
        let reloc_model = RelocMode::PIC;
        let code_model = CodeModel::Default;
        let opt_level = OptimizationLevel::Aggressive;
        let target_machine = target
            .create_target_machine(
                &target_triple,
                "generic",
                "",
                opt_level,
                reloc_model,
                code_model,
            )
            .unwrap();
        let file_type = FileType::Object;
        target_machine
            .write_to_file(&self.module, file_type, Path::new(path))
            .unwrap();

        let mut command = Command::new("link");
        command.arg(path).arg("/entry:main").arg("/out:main.exe").arg("ucrt.lib");
        let r = command.output().unwrap();
    }
}


fn main() {
    let path = std::env::args().nth(1).unwrap();
    let file = fs::read_to_string(path).unwrap();
    let mut lexer = Lexer::new(file.chars().collect());
    let ast = lexer.run();
    let ctx = Context::create();
    let mut cdg = CodeGen::new(&ctx, ast);
    cdg.run();
    cdg.generate_machine_code("main.o");
}
