//! SQLite 连接打开、读写分离与 PRAGMA 调优（零业务模块依赖）。

use rusqlite::Connection;

/// 采集写入的数据库入口。后台采集按文件短暂持有写锁、文件之间让出，
/// 用户操作（添加连接、清理数据等）不必等整轮扫描结束；测试直接传连接。
pub trait Store {
    fn write<R>(&mut self, work: impl FnOnce(&mut Connection) -> R) -> R;
}

impl Store for Connection {
    fn write<R>(&mut self, work: impl FnOnce(&mut Connection) -> R) -> R {
        work(self)
    }
}

impl Store for &std::sync::Mutex<Connection> {
    fn write<R>(&mut self, work: impl FnOnce(&mut Connection) -> R) -> R {
        let mut conn = self.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        work(&mut conn)
    }
}

/// 读写连接共用的调优：页缓存放得下整库常用页，临时排序走内存；
/// 读写分离后多个连接并发访问，写锁冲突时等待而不是立即报 busy。
pub(crate) fn tune(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "cache_size", -16_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(())
}

/// 打开主写连接并配置 WAL、外键与连接级调优（尚未执行建表迁移）。
pub(crate) fn open_raw(path: &std::path::Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    tune(&conn)?;
    Ok(conn)
}

/// 只读查询连接：统计、日志与实时快照走这里，不与采集写入争同一把锁。
/// WAL 模式下读者看到的是最近一次提交的一致快照；`query_only` 防止误写。
/// 必须在 [`super::open`] 完成建表与迁移之后再打开。
pub fn open_reader(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    tune(&conn)?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}
