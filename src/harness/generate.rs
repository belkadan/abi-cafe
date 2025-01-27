use camino::Utf8Path;
use camino::Utf8PathBuf;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::info;

use crate::error::*;
use crate::harness::test::*;
use crate::*;

impl TestHarness {
    pub async fn generate_test(&self, key: &TestKey) -> Result<GenerateOutput, GenerateError> {
        // FIXME: these two could be done concurrently
        let caller_src = self.generate_src(key, CallSide::Caller).await?;
        let callee_src = self.generate_src(key, CallSide::Callee).await?;

        Ok(GenerateOutput {
            caller_src,
            callee_src,
        })
    }

    async fn generate_src(
        &self,
        key: &TestKey,
        call_side: CallSide,
    ) -> Result<Utf8PathBuf, GenerateError> {
        let test = self
            .test_with_vals(&key.test, key.options.val_generator)
            .await?;
        let toolchain_id = key.toolchain_id(call_side).to_owned();
        let test_with_toolchain = self.test_with_toolchain(test, toolchain_id).await?;
        let src_path = self.src_path(key, call_side);

        // Briefly lock this map to insert/acquire a OnceCell and then release the lock
        let once = self
            .generated_sources
            .lock()
            .unwrap()
            .entry(src_path.clone())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone();
        // Either acquire the cached result, or make it
        let _ = once
            .get_or_try_init(|| {
                let toolchain = self.toolchain_by_test_key(key, call_side);
                let options = key.options.clone();
                info!("generating  {}", &src_path);
                generate_src(
                    &src_path,
                    toolchain,
                    test_with_toolchain,
                    call_side,
                    options,
                )
            })
            .await?;
        Ok(src_path)
    }

    fn src_path(&self, key: &TestKey, call_side: CallSide) -> Utf8PathBuf {
        let toolchain_id = key.toolchain_id(call_side);
        let toolchain = self.toolchain_by_test_key(key, call_side);
        let mut output = self.base_id(key, Some(call_side), "_");
        output.push('.');
        output.push_str(toolchain.src_ext());
        self.paths.generated_src_dir.join(toolchain_id).join(output)
    }
}

async fn generate_src(
    src_path: &Utf8Path,
    toolchain: Arc<dyn Toolchain + Send + Sync>,
    test_with_toolchain: Arc<TestWithToolchain>,
    call_side: CallSide,
    options: TestOptions,
) -> Result<(), GenerateError> {
    let mut output_string = String::new();
    let test = test_with_toolchain.with_options(options)?;
    match call_side {
        CallSide::Callee => toolchain.generate_callee(&mut output_string, test)?,
        CallSide::Caller => toolchain.generate_caller(&mut output_string, test)?,
    }

    // Write the result to disk
    std::fs::create_dir_all(src_path.parent().expect("source file had no parent!?"))?;
    let mut output = File::create(src_path)?;
    output.write_all(output_string.as_bytes())?;

    Ok(())
}
