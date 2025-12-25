pub use read::{ReadHandler, ReadPlugin};
pub use write::{WriteHandler, WritePlugin};

// HACK: `FileHandle` is not `Send` on wasm32, so we need to wrap it in a type
// that is so that we can use it in `TaskRunner`.
struct SendableFileHandle(rfd::FileHandle);
unsafe impl Send for SendableFileHandle {}

mod read {
    use bevy::prelude::*;
    use bevy_async_task::TaskRunner;
    use std::task::Poll;

    use super::SendableFileHandle;

    pub trait ReadHandler: Sync + Send + Clone + 'static {
        /// Event to trigger the read process. Will be registered for you.
        type Event: Message;

        /// Returns the filename to use for the load dialog. If `None`, the
        /// dialog will use a default filename.
        fn filename(&self, world: &World) -> Option<String>;
        /// Called when the user closes the load dialog without selecting a file.
        fn on_load_without_filename(&self, world: &mut World);
        /// Called when the file has been read and the buffer is available.
        fn read(&self, world: &mut World, buffer: Vec<u8>);
    }

    pub struct ReadPlugin<R: ReadHandler> {
        pub handler: R,
    }
    impl<R: ReadHandler> ReadPlugin<R> {
        pub fn new(handler: R) -> Self {
            Self { handler }
        }
    }
    impl<R: ReadHandler> Plugin for ReadPlugin<R> {
        fn build(&self, app: &mut App) {
            app.add_message::<R::Event>()
                .add_systems(PostUpdate, build_load_system(self.handler.clone()));
        }
    }
    fn build_load_system<R: ReadHandler>(
        handler: R,
    ) -> impl Fn(&mut World, TaskRunner<Option<SendableFileHandle>>, TaskRunner<Vec<u8>>) {
        move |world: &mut World,
              mut file_dialog_executor: TaskRunner<Option<SendableFileHandle>>,
              mut read_executor: TaskRunner<Vec<u8>>| {
            if file_dialog_executor.is_idle() {
                if world.resource_mut::<Messages<R::Event>>().drain().count() > 0 {
                    let async_file_dialog = rfd::AsyncFileDialog::new();
                    let async_file_dialog = if let Some(filename) = handler.filename(world) {
                        async_file_dialog.set_file_name(filename)
                    } else {
                        async_file_dialog
                    };
                    file_dialog_executor.start(async move {
                        async_file_dialog.pick_file().await.map(SendableFileHandle)
                    });
                }
            } else if let Poll::Ready(h) = file_dialog_executor.poll() {
                let Some(SendableFileHandle(file_handle)) = h else {
                    handler.on_load_without_filename(world);
                    return;
                };

                read_executor.start(async move { file_handle.read().await });
            }

            if let Poll::Ready(buffer) = read_executor.poll() {
                handler.read(world, buffer);
            }
        }
    }
}

mod write {
    use bevy::prelude::*;
    use bevy_async_task::TaskRunner;
    use std::task::Poll;

    use super::SendableFileHandle;

    pub trait WriteHandler: Sync + Send + Clone + 'static {
        /// Event to trigger the write process. Will be registered for you.
        type Event: Message;

        /// Returns the filename to use for the save dialog. If `None`, the
        /// dialog will use a default filename.
        fn filename(&self, world: &World) -> Option<String>;
        /// Called when the user closes the save dialog without selecting a file.
        fn on_save_without_filename(&self, world: &mut World);
        /// Called when the file is available to write to. Should return the
        /// buffer to write.
        fn write(&self, world: &mut World) -> Vec<u8>;
        /// Called when the write is complete and a result is available.
        fn on_write_complete(&self, world: &mut World, result: std::io::Result<()>);
    }

    pub struct WritePlugin<W: WriteHandler> {
        pub handler: W,
    }
    impl<W: WriteHandler> WritePlugin<W> {
        pub fn new(handler: W) -> Self {
            Self { handler }
        }
    }
    impl<W: WriteHandler> Plugin for WritePlugin<W> {
        fn build(&self, app: &mut App) {
            app.add_message::<W::Event>()
                .add_systems(PostUpdate, build_save_system(self.handler.clone()));
        }
    }
    fn build_save_system<W: WriteHandler>(
        handler: W,
    ) -> impl Fn(&mut World, TaskRunner<Option<SendableFileHandle>>, TaskRunner<std::io::Result<()>>)
    {
        move |world: &mut World,
              mut file_dialog_executor: TaskRunner<Option<SendableFileHandle>>,
              mut write_executor: TaskRunner<std::io::Result<()>>| {
            if file_dialog_executor.is_idle() {
                if world.resource_mut::<Messages<W::Event>>().drain().count() > 0 {
                    let async_file_dialog = rfd::AsyncFileDialog::new();
                    let async_file_dialog = if let Some(filename) = handler.filename(world) {
                        async_file_dialog.set_file_name(filename)
                    } else {
                        async_file_dialog
                    };
                    file_dialog_executor.start(async move {
                        async_file_dialog.save_file().await.map(SendableFileHandle)
                    });
                }
            } else if let Poll::Ready(h) = file_dialog_executor.poll() {
                let Some(SendableFileHandle(file_handle)) = h else {
                    handler.on_save_without_filename(world);
                    return;
                };

                let buffer = handler.write(world);
                write_executor.start(async move { file_handle.write(&buffer).await });
            }

            if let Poll::Ready(r) = write_executor.poll() {
                handler.on_write_complete(world, r);
            }
        }
    }
}
