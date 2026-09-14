//! Core codegen unit tests: emission, jumps, peephole, interning.

use carnelian_compiler::codegen::{LoopType, Scope, Session};
use carnelian_compiler::opcode;

fn new_scope() -> (Session, Scope) {
    let mut session = Session::new();
    let child = Scope::child(&mut session, &Scope::top(), &[]).expect("child scope");
    (session, child)
}

#[test]
fn genop_operand_widths() {
    let (session, mut scope) = new_scope();
    scope
        .genop_2(&session, opcode::OP_SSEND, 1, 2)
        .expect("genop");
    assert_eq!(&scope.iseq[..scope.pc as usize], &[opcode::OP_SSEND, 1, 2]);

    let (session, mut scope) = new_scope();
    scope
        .genop_2(&session, opcode::OP_SSEND, 1, 0x1ff)
        .expect("genop");
    assert_eq!(
        &scope.iseq[..scope.pc as usize],
        &[opcode::OP_EXT2, opcode::OP_SSEND, 1, 0x01, 0xff]
    );

    let (session, mut scope) = new_scope();
    scope
        .genop_2(&session, opcode::OP_SSEND, 0x100, 0x1ff)
        .expect("genop");
    assert_eq!(
        &scope.iseq[..scope.pc as usize],
        &[opcode::OP_EXT3, opcode::OP_SSEND, 0x01, 0x00, 0x01, 0xff]
    );
}

#[test]
fn gen_int_boundaries() {
    let cases: &[(i64, &[u8])] = &[
        (-0x8000_0001, &[opcode::OP_LOADL, 0, 0]), // pool index patched below
        (-0x8000, &[opcode::OP_LOADI16, 0, 0x80, 0x00]),
        (-0xff, &[opcode::OP_LOADINEG, 0, 0xff]),
        (-1, &[opcode::OP_LOADI__1, 0]),
        (0, &[opcode::OP_LOADI_0, 0]),
        (7, &[opcode::OP_LOADI_7, 0]),
        (8, &[opcode::OP_LOADI8, 0, 8]),
        (0xff, &[opcode::OP_LOADI8, 0, 0xff]),
        (0x100, &[opcode::OP_LOADI16, 0, 0x01, 0x00]),
        (0x7fff, &[opcode::OP_LOADI16, 0, 0x7f, 0xff]),
        (0x8000, &[opcode::OP_LOADI32, 0, 0x00, 0x00, 0x80, 0x00]),
        (
            0x7fff_ffff,
            &[opcode::OP_LOADI32, 0, 0x7f, 0xff, 0xff, 0xff],
        ),
        (0x8000_0000, &[opcode::OP_LOADL, 0, 0]),
    ];
    for (value, prefix) in cases {
        let (mut session, mut scope) = new_scope();
        scope.push_n(3).expect("push");
        scope.gen_int(&mut session, 0, *value).expect("gen_int");
        let emitted = &scope.iseq[..scope.pc as usize];
        assert_eq!(
            &emitted[..prefix.len().min(2)],
            &prefix[..prefix.len().min(2)]
        );
        assert_eq!(emitted[0], prefix[0], "opcode for {value}");
        if emitted[0] == opcode::OP_LOADL {
            assert_eq!(emitted.len(), 3);
        } else {
            assert_eq!(emitted, *prefix, "bytes for {value}");
        }
    }
}

#[test]
fn move_fuses_preceding_load() {
    let (mut session, mut scope) = new_scope();
    scope.nlocals = 1;
    scope
        .genop_1(&session, opcode::OP_LOADI_1, 5)
        .expect("load");
    scope.gen_move(&mut session, 2, 5, false).expect("move");
    assert_eq!(&scope.iseq[..scope.pc as usize], &[opcode::OP_LOADI_1, 2]);
}

#[test]
fn move_to_same_register_is_dropped() {
    let (mut session, mut scope) = new_scope();
    scope
        .genop_1(&session, opcode::OP_LOADI_1, 5)
        .expect("load");
    let before = scope.pc;
    scope.gen_move(&mut session, 5, 5, false).expect("move");
    assert_eq!(scope.pc, before);
}

#[test]
fn jump_patch_roundtrip() {
    let (session, mut scope) = new_scope();
    let pos = scope
        .genjmp2(&session, opcode::OP_JMPNOT, 1, u32::MAX, false)
        .expect("jmp");
    scope
        .genop_1(&session, opcode::OP_LOADNIL, 1)
        .expect("load");
    scope.dispatch(pos).expect("patch");
    // JMPNOT R1 +2 over the 2-byte LOADNIL.
    assert_eq!(
        &scope.iseq[..scope.pc as usize],
        &[opcode::OP_JMPNOT, 1, 0, 2, opcode::OP_LOADNIL, 1]
    );
}

#[test]
fn int_pool_dedups_both_widths() {
    let (session, mut scope) = new_scope();
    let first = scope.new_lit_int(&session, 3_000_000_000).expect("int");
    let second = scope.new_lit_int(&session, 3_000_000_000).expect("int");
    assert_eq!(first, second);
    let small = scope.new_lit_int(&session, 7).expect("int");
    assert_eq!(scope.new_lit_int(&session, 7).expect("int"), small);
}

#[test]
fn str_pool_dedups_and_rejects_huge() {
    let (session, mut scope) = new_scope();
    let first = scope.new_lit_str(&session, b"hi").expect("str");
    assert_eq!(scope.new_lit_str(&session, b"hi").expect("str"), first);
    assert_ne!(scope.new_lit_str(&session, b"bye").expect("str"), first);
    let huge = vec![b'x'; 0x1_0000];
    assert!(scope.new_lit_str(&session, &huge).is_err());
}

#[test]
fn float_pool_distinguishes_zero_sign() {
    let (session, mut scope) = new_scope();
    let pos = scope.new_lit_float(&session, 0.0).expect("float");
    assert_eq!(scope.new_lit_float(&session, 0.0).expect("float"), pos);
    assert_ne!(scope.new_lit_float(&session, -0.0).expect("float"), pos);
}

#[test]
fn bigint_raw_layout_matches_dump() {
    let (session, mut scope) = new_scope();
    let index = scope
        .new_litbint(&session, b"99", 10, false)
        .expect("bigint");
    assert_eq!(
        scope
            .new_litbint(&session, b"99", 10, false)
            .expect("bigint"),
        index
    );
    assert_ne!(
        scope
            .new_litbint(&session, b"99", 10, true)
            .expect("bigint"),
        index
    );
    match &scope.pool[index] {
        carnelian_compiler::codegen::CgPool::BigInt(raw) => {
            assert_eq!(raw, &[2, 10, b'9', b'9']);
        }
        other => panic!("expected bigint, got {other:?}"),
    }
}

#[test]
fn syms_keep_first_seen_order() {
    let (mut session, mut scope) = new_scope();
    assert_eq!(scope.new_sym(&mut session, b"puts").expect("sym"), 0);
    assert_eq!(scope.new_sym(&mut session, b"puts").expect("sym"), 0);
    assert_eq!(scope.new_sym(&mut session, b"p").expect("sym"), 1);
}

#[test]
fn return_fuses_loadnil() {
    let (session, mut scope) = new_scope();
    scope
        .genop_1(&session, opcode::OP_LOADNIL, 1)
        .expect("load");
    scope
        .gen_return(&session, opcode::OP_RETURN, 1)
        .expect("ret");
    assert_eq!(&scope.iseq[..scope.pc as usize], &[opcode::OP_RETNIL]);
}

#[test]
fn finish_computes_counts_and_lvar() {
    let mut session = Session::new();
    let child = Scope::child(&mut session, &Scope::top(), &[b"x".to_vec()]).expect("child");
    let irep = child.finish(&mut session).expect("finish");
    assert_eq!(irep.nlocals, 2);
    assert_eq!(irep.nregs, 2);
    assert_eq!(irep.lv, vec![Some(0)]);
}

#[test]
fn loop_pop_patches_break_chain() {
    let (session, mut scope) = new_scope();
    scope.loop_push(LoopType::Normal);
    let held = scope.genjmp(opcode::OP_JMP, u32::MAX).expect("jmp");
    scope.loops.last_mut().expect("frame").pc2 = held;
    scope
        .genop_1(&session, opcode::OP_LOADNIL, 0)
        .expect("load");
    scope.loop_pop(&session, false).expect("pop");
    assert!(scope.loops.is_empty());
}
