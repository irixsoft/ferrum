pub const MAX_TEXT_BYTES: u64 = 512 * 1024;

const SKIPPED_DIRS: [&str; 10] = [
    ".git",
    "node_modules",
    "dist",
    ".next",
    "build",
    "out",
    "coverage",
    ".turbo",
    ".cache",
    "vendor",
];
const TEXT_EXTENSIONS: [&str; 12] = [
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "vue", "svelte", "astro", "cs",
];

pub fn skipped_dir(name: &str) -> bool {
    SKIPPED_DIRS.contains(&name)
}

/// Whether a path relative to the scanned root is source worth reading.
pub fn wanted_text_file(relative: &str) -> bool {
    let mut parts = relative.split('/').peekable();
    let mut name = "";
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            if skipped_dir(part) {
                return false;
            }
        } else {
            name = part;
        }
    }
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| TEXT_EXTENSIONS.contains(&ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_files_are_wanted_and_build_output_is_not() {
        for path in [
            "src/env.ts",
            "app/page.tsx",
            "server.mjs",
            "Api/Program.cs",
            "components/x.vue",
        ] {
            assert!(wanted_text_file(path), "{path}");
        }
        for path in [
            "node_modules/pg/index.js",
            ".next/server/app.js",
            "dist/index.js",
            "src/.git/x.ts",
            "README.md",
            "package.json",
            "public/logo.svg",
            "src/styles.css",
        ] {
            assert!(!wanted_text_file(path), "{path}");
        }
    }
}
