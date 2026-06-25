use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use crate::cache::CachedConfig;
use crate::config::Config;

/// 全局上下文，保存所有命令共享的配置和标志
#[derive(Clone)]
pub struct GlobalContext {
    pub no_color: bool,
    pub use_cache: bool,
    pub no_save: bool,
    pub profile: Option<String>,
    pub config: Config,
}

/// 针对支持缓存的命令，如 Get, Download, Bench
#[async_trait]
pub trait CacheableAction {
    type Args: Clone + Send + Sync;
    type Merged: Send + Sync;

    fn get_provided_args(&self) -> Self::Args;
    fn detect_conflicts(&self, cache: &CachedConfig, args: &Self::Args) -> Vec<String>;
    fn merge_config(&self, cache: &CachedConfig, args: &Self::Args) -> Self::Merged;
    fn has_new_params(&self, args: &Self::Args) -> bool;
    fn update_cache(&self, cache: &mut CachedConfig, args: &Self::Args, ctx: &GlobalContext);
    async fn run(&self, merged: Self::Merged, ctx: &GlobalContext) -> Result<()>;

    /// 统一处理缓存加载、冲突检测、合并和保存的执行流程
    async fn execute(&self, ctx: &GlobalContext) -> Result<()> {
        let cache_path = CachedConfig::get_cache_path();
        let mut cached_config = CachedConfig::load_from_file(&cache_path)?;

        if ctx.use_cache {
            if cached_config.is_empty() {
                eprintln!("Error: No cached configuration found. Please run a command without -x first to create a cache.");
                return Ok(());
            }
            let args = self.get_provided_args();
            let conflicts = self.detect_conflicts(&cached_config, &args);
            if !conflicts.is_empty() {
                eprintln!("Error: Configuration conflicts detected when using cache:");
                for conflict in conflicts {
                    eprintln!(" - {}", conflict);
                }
                eprintln!("Please resolve conflicts or run without -x to override cache.");
                return Ok(());
            }
            let merged = self.merge_config(&cached_config, &args);
            if self.has_new_params(&args) {
                self.update_cache(&mut cached_config, &args, ctx);
                cached_config.save_to_file(&cache_path)?;
                crate::log::log_info("Updated cache with new parameters");
            }
            crate::log::log_info("Using cached configuration");
            self.run(merged, ctx).await
        } else {
            let args = self.get_provided_args();
            // 使用默认缓存来获取默认值
            let merged = self.merge_config(&CachedConfig::default(), &args);
            let result = self.run(merged, ctx).await;
            if !ctx.no_save && result.is_ok() {
                self.update_cache(&mut cached_config, &args, ctx);
                cached_config.save_to_file(&cache_path)?;
                crate::log::log_info("Configuration saved to cache");
            }
            result
        }
    }
}

/// 针对修改配置和 Profile 的子命令
#[async_trait]
pub trait ConfigActionHandler {
    async fn handle(&self, config: &mut Config, config_path: &PathBuf) -> Result<()>;
}

/// 针对历史记录和断点续传管理的子命令
#[async_trait]
pub trait SimpleActionHandler {
    async fn handle(&self) -> Result<()>;
}
