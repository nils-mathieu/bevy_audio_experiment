use {
    bevy_asset::{AssetLoader, LoadContext, io::Reader, prelude::*},
    bevy_reflect::prelude::*,
    bevy_tasks::ConditionalSendFuture,
    std::sync::Arc,
};

#[derive(TypePath, Asset)]
pub struct AudioFile(pub(crate) Arc<[u8]>);

#[derive(TypePath, Default)]
pub struct AudioFileLoader;

impl AssetLoader for AudioFileLoader {
    type Asset = AudioFile;
    type Settings = ();
    type Error = std::io::Error;

    fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext,
    ) -> impl ConditionalSendFuture<Output = Result<Self::Asset, Self::Error>> {
        async move {
            let mut output = Vec::new();
            reader.read_to_end(&mut output).await?;
            Ok(AudioFile(Arc::from(output)))
        }
    }

    fn extensions(&self) -> &[&str] {
        &[
            #[cfg(feature = "wav")]
            "wav",
        ]
    }
}
