use ferrum_platform::ubuntu::PG_PORT;

pub const MAX_CONNECTIONS: u32 = 100;

const KB_PER_MB: u64 = 1024;
const KB_PER_GB: u64 = 1024 * 1024;
const WORKER_PROCESSES: u64 = 8;
const MIN_WORK_MEM_KB: u64 = 4 * KB_PER_MB;
const MAINTENANCE_CAP_KB: u64 = 8 * KB_PER_GB;
const MAX_WAL_BUFFERS_KB: u64 = 16 * KB_PER_MB;
const NEAR_WAL_BUFFERS_KB: u64 = 14 * KB_PER_MB;
const MIN_WAL_BUFFERS_KB: u64 = 32;
const HUGE_PAGES_FROM_KB: u64 = 2 * KB_PER_GB;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    pub max_connections: u32,
    pub shared_buffers_kb: u64,
    pub effective_cache_size_kb: u64,
    pub maintenance_work_mem_kb: u64,
    pub work_mem_kb: u64,
    pub wal_buffers_kb: u64,
    pub huge_pages: bool,
}

/// pgtune's web-application profile on SSD, with the parallel workers left at PostgreSQL's
/// defaults.
pub fn tuning(total_ram_kb: u64, max_connections: u32) -> Tuning {
    let shared_buffers_kb = total_ram_kb / 4;
    let maintenance_work_mem_kb = (total_ram_kb / 16).min(MAINTENANCE_CAP_KB);
    let consumers = (u64::from(max_connections) + WORKER_PROCESSES) * 3;
    let work_mem_kb =
        (total_ram_kb.saturating_sub(shared_buffers_kb) / consumers).max(MIN_WORK_MEM_KB);
    let mut wal_buffers_kb = (shared_buffers_kb * 3 / 100).min(MAX_WAL_BUFFERS_KB);
    if wal_buffers_kb > NEAR_WAL_BUFFERS_KB {
        wal_buffers_kb = MAX_WAL_BUFFERS_KB;
    }
    Tuning {
        max_connections,
        shared_buffers_kb,
        effective_cache_size_kb: total_ram_kb * 3 / 4,
        maintenance_work_mem_kb,
        work_mem_kb,
        wal_buffers_kb: wal_buffers_kb.max(MIN_WAL_BUFFERS_KB),
        huge_pages: shared_buffers_kb >= HUGE_PAGES_FROM_KB,
    }
}

pub fn render_conf(t: &Tuning) -> String {
    let mut out = String::new();
    out.push_str("listen_addresses = '127.0.0.1'\n");
    out.push_str(&format!("port = {PG_PORT}\n"));
    out.push_str(&format!("max_connections = {}\n", t.max_connections));
    out.push_str(&format!("shared_buffers = {}\n", mem(t.shared_buffers_kb)));
    out.push_str(&format!(
        "effective_cache_size = {}\n",
        mem(t.effective_cache_size_kb)
    ));
    out.push_str(&format!(
        "maintenance_work_mem = {}\n",
        mem(t.maintenance_work_mem_kb)
    ));
    out.push_str(&format!("work_mem = {}\n", mem(t.work_mem_kb)));
    out.push_str(&format!("wal_buffers = {}\n", mem(t.wal_buffers_kb)));
    out.push_str(&format!(
        "huge_pages = {}\n",
        if t.huge_pages { "try" } else { "off" }
    ));
    out.push_str("checkpoint_completion_target = 0.9\n");
    out.push_str("min_wal_size = 1GB\n");
    out.push_str("max_wal_size = 4GB\n");
    out.push_str("default_statistics_target = 100\n");
    out.push_str("random_page_cost = 1.1\n");
    out.push_str("effective_io_concurrency = 200\n");
    out.push_str("jit = off\n");
    out
}

fn mem(kb: u64) -> String {
    if kb.is_multiple_of(KB_PER_MB) {
        format!("{}MB", kb / KB_PER_MB)
    } else {
        format!("{kb}kB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_gigabyte_box_gets_pgtune_shaped_numbers() {
        let t = tuning(2 * KB_PER_GB, 100);
        assert_eq!(t.shared_buffers_kb, 512 * KB_PER_MB);
        assert_eq!(t.effective_cache_size_kb, 1536 * KB_PER_MB);
        assert_eq!(t.maintenance_work_mem_kb, 128 * KB_PER_MB);
        assert_eq!(t.work_mem_kb, 4854);
        assert_eq!(t.wal_buffers_kb, 16 * KB_PER_MB);
        assert_eq!(t.max_connections, 100);
        assert!(!t.huge_pages);
    }

    #[test]
    fn small_boxes_keep_the_floors_and_big_ones_the_caps() {
        let small = tuning(512 * KB_PER_MB, 100);
        assert_eq!(small.work_mem_kb, MIN_WORK_MEM_KB);
        assert_eq!(small.wal_buffers_kb, 3932);
        let big = tuning(256 * KB_PER_GB, 100);
        assert_eq!(big.maintenance_work_mem_kb, MAINTENANCE_CAP_KB);
        assert_eq!(big.wal_buffers_kb, MAX_WAL_BUFFERS_KB);
        assert!(big.huge_pages);
    }

    #[test]
    fn the_rendered_conf_binds_loopback_and_leaves_no_placeholder() {
        let conf = render_conf(&tuning(4 * KB_PER_GB, 100));
        assert!(conf.starts_with("listen_addresses = '127.0.0.1'\nport = 5432\n"));
        assert!(conf.contains("shared_buffers = 1024MB\n"));
        assert!(conf.contains("effective_cache_size = 3072MB\n"));
        assert!(conf.contains("work_mem = 9709kB\n"));
        assert!(conf.contains("random_page_cost = 1.1\n"));
        assert!(conf.contains("huge_pages = off\n"));
        assert!(render_conf(&tuning(16 * KB_PER_GB, 100)).contains("huge_pages = try\n"));
        assert!(!conf.contains("{{"));
    }
}
