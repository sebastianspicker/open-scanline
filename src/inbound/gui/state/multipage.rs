use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn take_multipage_save_candidate(
        &mut self,
        destination: &Path,
        source: &Path,
    ) -> (MultipageSaveCandidate, Option<String>) {
        let session = self
            .multipage_session
            .as_ref()
            .filter(|session| session.destination == destination);
        let mut sources = session.map_or_else(Vec::new, |session| session.sources.clone());
        sources.push(source.to_path_buf());
        let (export, pending_pdf_password) = match session {
            Some(session) => (session.export.clone(), None),
            None => {
                let export = self.take_export_options();
                let pending_pdf_password = export.pdf_password.clone();
                (export, pending_pdf_password)
            }
        };
        (
            MultipageSaveCandidate {
                destination: destination.to_path_buf(),
                sources,
                export,
            },
            pending_pdf_password,
        )
    }

    /// Install an export candidate only after its complete rebuilt container
    /// was published, then consume the runtime-only password.
    pub(in crate::inbound::gui) fn commit_multipage_save(
        &mut self,
        candidate: MultipageSaveCandidate,
    ) {
        self.multipage_session = Some(MultipageExportSession {
            destination: candidate.destination,
            sources: candidate.sources,
            export: candidate.export,
        });
        self.pdf_password.clear();
    }

    pub(in crate::inbound::gui) fn clear_multipage_session(&mut self) {
        self.multipage_session = None;
    }

    #[cfg(feature = "gui")]
    pub(in crate::inbound::gui) fn references_working_source(&self, path: &Path) -> bool {
        self.last_image.as_deref() == Some(path)
            || self
                .multipage_session
                .as_ref()
                .is_some_and(|session| session.sources.iter().any(|source| source == path))
    }
}
