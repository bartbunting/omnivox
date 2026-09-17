use super::*;
use crate::native_parameters::{
    CatalogueIdentity, CommonMapping, ParameterCatalogue, ParameterDescriptor, MAX_PARAMETERS,
};
use std::collections::BTreeSet;

/// Assemble pages from one engine/voice/runtime. An invalid page never changes
/// accepted state. Busy/unavailable results are handled by the query owner.
#[derive(Debug)]
pub struct CatalogueAssembly {
    query: CatalogueQuery,
    identity: Option<CatalogueIdentity>,
    mappings: Vec<CommonMapping>,
    parameters: Vec<ParameterDescriptor>,
    cursors: BTreeSet<String>,
    complete: bool,
}
impl CatalogueAssembly {
    pub fn new(query: CatalogueQuery) -> Result<Self, HelperProtocolError> {
        query.validate()?;
        require(query.cursor.is_none(), "initial_cursor")?;
        Ok(Self {
            query,
            identity: None,
            mappings: Vec::new(),
            parameters: Vec::new(),
            cursors: BTreeSet::new(),
            complete: false,
        })
    }
    pub fn next_query(&self) -> Option<&CatalogueQuery> {
        (!self.complete).then_some(&self.query)
    }

    /// The caller correlates the response envelope with Request::request_id and
    /// passes the submitted query, so stale concurrent replies cannot advance it.
    pub fn push(
        &mut self,
        query: &CatalogueQuery,
        page: &CatalogueResult,
    ) -> Result<(), HelperProtocolError> {
        require(!self.complete && query == &self.query, "page_query")?;
        page.validate()?;
        let CatalogueResult::Ready {
            identity,
            voice_id,
            parameters,
            mappings,
            next_cursor,
        } = page
        else {
            return Err(invalid("page_status"));
        };
        require(
            voice_id == &query.voice_id
                && query
                    .expected_catalogue_revision
                    .as_ref()
                    .is_none_or(|r| r == &identity.catalogue_revision),
            "page_identity",
        )?;
        require(
            self.identity
                .as_ref()
                .is_none_or(|i| i == identity && &self.mappings == mappings),
            "page_identity",
        )?;
        require(
            self.parameters.len() + parameters.len() <= MAX_PARAMETERS,
            "catalogue_parameters",
        )?;
        require(
            next_cursor
                .as_ref()
                .is_none_or(|c| !self.cursors.contains(c)),
            "repeated_cursor",
        )?;
        let mut accumulated = self.parameters.clone();
        accumulated.extend_from_slice(parameters);
        let unique = accumulated.iter().map(|p| &p.id).collect::<BTreeSet<_>>();
        require(unique.len() == accumulated.len(), "duplicate_parameter")?;
        if next_cursor.is_none() {
            ParameterCatalogue {
                engine_id: query.engine_id.clone(),
                identity: identity.clone(),
                voice_id: voice_id.clone(),
                parameters: accumulated.clone(),
                mappings: mappings.clone(),
            }
            .validate()
            .map_err(|_| invalid("complete_catalogue"))?;
        }
        self.parameters = accumulated;
        self.identity = Some(identity.clone());
        self.mappings = mappings.clone();
        self.query.expected_catalogue_revision = Some(identity.catalogue_revision.clone());
        self.query.cursor = next_cursor.clone();
        if let Some(c) = next_cursor {
            self.cursors.insert(c.clone());
        }
        self.complete = next_cursor.is_none();
        Ok(())
    }
    pub fn finish(self) -> Result<ParameterCatalogue, HelperProtocolError> {
        require(self.complete, "incomplete_catalogue")?;
        Ok(ParameterCatalogue {
            engine_id: self.query.engine_id,
            identity: self.identity.ok_or_else(|| invalid("identity"))?,
            voice_id: self.query.voice_id,
            parameters: self.parameters,
            mappings: self.mappings,
        })
    }
}
