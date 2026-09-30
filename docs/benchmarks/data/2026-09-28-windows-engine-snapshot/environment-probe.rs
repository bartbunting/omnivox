use std::process::Command;
fn main() {
    let variables: Vec<_> = std::env::vars_os().collect();
    let mut command = Command::new("unused");
    command.env_clear().envs(variables.iter().map(|(n,v)|(n,v)));
    println!("captured={}, launch_keys={}", variables.len(), command.get_envs().count());
    for (name, value) in &variables {
        if name.is_empty() || name.as_encoded_bytes().contains(&b'=') || name.as_encoded_bytes().contains(&0) || value.as_encoded_bytes().contains(&0) {
            println!("invalid entry: name_len={}, starts_equals={}, contains_nul={}", name.len(), name.as_encoded_bytes().starts_with(b"="), value.as_encoded_bytes().contains(&0));
        }
    }
}
