#[test]
fn test_verify_simple() {
    use x3_lang_vm::verifier::verify;
    use x3_lang_vm::InstructionStream;

    let code = vec![
        0x20u8, 0x00, 0x01, 0x00, // PUSH_IMM 1
        0x20u8, 0x00, 0x02, 0x00, // PUSH_IMM 2
        0x01u8, 0x00, 0x00, 0x00, // ADD_RRR
        0xFFu8, 0x00, 0x00, 0x00, // HALT
    ];
    let stream = InstructionStream::new(code);
    let res = verify(&stream);
    assert!(res.is_ok());
}
