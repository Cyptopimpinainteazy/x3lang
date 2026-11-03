use x3_lang_compiler::parser::parse_source;

#[test]
fn test_parse_user_example() {
    let source = include_str!("../../examples/mainnet_safe_swap.x3");
    match parse_source(source) {
        Ok(program) => {
            println!("PARSED OK. Items count: {}", program.items.len());
            for item in &program.items {
                let variant = match &item.node {
                    x3_lang_ast::ast::Item::VmDecl(_) => "VmDecl",
                    x3_lang_ast::ast::Item::SolverMarket(_) => "SolverMarket",
                    x3_lang_ast::ast::Item::RelayerSwarm(_) => "RelayerSwarm",
                    x3_lang_ast::ast::Item::RpcQuorum(_) => "RpcQuorum",
                    x3_lang_ast::ast::Item::RiskPolicy(_) => "RiskPolicy",
                    x3_lang_ast::ast::Item::PrivacyBlock(_) => "PrivacyBlock",
                    x3_lang_ast::ast::Item::InvariantDecl(_) => "InvariantDecl",
                    x3_lang_ast::ast::Item::ProofsRequired(_) => "ProofsRequired",
                    x3_lang_ast::ast::Item::FinalityPolicy(_) => "FinalityPolicy",
                    x3_lang_ast::ast::Item::ErrorDecl(_) => "ErrorDecl",
                    x3_lang_ast::ast::Item::VmTarget(_) => "VmTarget",
                    x3_lang_ast::ast::Item::IntentDecl(_) => "IntentDecl",
                    _ => "Other",
                };
                println!("  - {}", variant);
            }
        }
        Err(e) => {
            panic!("PARSE ERROR: {:?}", e);
        }
    }
}
