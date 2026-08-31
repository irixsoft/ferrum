use crate::Platform;

pub struct Ubuntu;

impl Platform for Ubuntu {
    fn resolve_package(&self, name: &str) -> Vec<String> {
        vec![name.to_string()]
    }
}
