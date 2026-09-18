//! Native execution and tentative evidence for one routed synthesis attempt.
use super::*;
use omnivox_tts::engine_voice_choices::NativeChoiceExecution;
use omnivox_tts::native_synthesis::{validate_application, ApplicationStatus, NativeApplication};
use std::cell::RefCell;

pub(super) fn definition_payload_bytes(definition: &EngineLayeredVoiceDefinition) -> usize {
    // Include encoded strings and values plus conservative per-entry allocation
    // overhead. Native maps must count toward the existing queue byte budget.
    definition.choices.iter().fold(
        serde_json::to_vec(definition)
            .map_or(usize::MAX, |v| v.len())
            .saturating_add(std::mem::size_of::<EngineLayeredVoiceDefinition>()),
        |bytes, choice| {
            bytes
                .saturating_add(std::mem::size_of_val(choice))
                .saturating_add(choice.native.as_ref().map_or(0, |native| {
                    native.parameters.len().saturating_mul(
                        std::mem::size_of::<(
                            String,
                            omnivox_tts::voice_choices::Adjustment<
                                omnivox_tts::native_parameters::NativeValue,
                            >,
                        )>() + 8 * std::mem::size_of::<usize>(),
                    )
                }))
        },
    )
}

fn degraded(reason: &str) -> Result<NativeApplication, TtsError> {
    let application = NativeApplication {
        status: ApplicationStatus::CommonOnly,
        plan_id: None,
        identity: None,
        masked_parameters: vec![],
        reason: Some(reason.into()),
    };
    application
        .validate()
        .map_err(|e| TtsError::InvalidParameter(e.to_string()))?;
    Ok(application)
}

pub(super) fn synthesize_buffered(
    engine: &dyn TtsEngine,
    request: &SynthesisRequest,
    prepared: &mut choice::PreparedVoiceAttempt,
) -> Result<SynthesisResult, TtsError> {
    let (result, application) = match &prepared.native {
        NativeChoiceExecution::NotRequested => (engine.synthesize(request)?, None),
        NativeChoiceExecution::CommonOnly { reason } => {
            let application = degraded(reason)?;
            (engine.synthesize(request)?, Some(application))
        }
        NativeChoiceExecution::Parameters(parameters) => {
            let (result, application) = engine.synthesize_with_parameters(request, parameters)?;
            validate_application(
                &prepared.resolution.realized.engine_id,
                parameters,
                &application,
            )?;
            (result, Some(application))
        }
    };
    prepared.native_application = application;
    Ok(result)
}

enum Receipt {
    Pending,
    Ready(NativeApplication),
    Invalid(String),
}

/// Interior mutability only connects the synchronous receipt callback and sink.
/// This state never crosses an engine thread or survives the current attempt.
pub(super) struct ReceiptState<'a> {
    native: &'a NativeChoiceExecution,
    engine_id: &'a str,
    generation: u64,
    generation_counter: &'a AtomicU64,
    cancellation: Option<&'a SynthesisCancellationToken>,
    receipt: RefCell<Receipt>,
}

impl<'a> ReceiptState<'a> {
    pub(super) fn new(
        native: &'a NativeChoiceExecution,
        engine_id: &'a str,
        generation: u64,
        generation_counter: &'a AtomicU64,
        cancellation: Option<&'a SynthesisCancellationToken>,
    ) -> Self {
        Self {
            native,
            engine_id,
            generation,
            generation_counter,
            cancellation,
            receipt: RefCell::new(Receipt::Pending),
        }
    }

    pub(super) fn record(&self, application: &NativeApplication) {
        let mut receipt = self.receipt.borrow_mut();
        if !matches!(*receipt, Receipt::Pending) {
            *receipt = Receipt::Invalid("repeated native application receipt".into());
            return;
        }
        let NativeChoiceExecution::Parameters(parameters) = self.native else {
            *receipt = Receipt::Invalid("unexpected native application receipt".into());
            return;
        };
        *receipt = match validate_application(self.engine_id, parameters, application) {
            Ok(()) => Receipt::Ready(application.clone()),
            Err(error) => Receipt::Invalid(error.to_string()),
        };
    }

    pub(super) fn check_and_attach(
        &self,
        application: &mut Option<NativeApplication>,
    ) -> Result<(), TtsError> {
        if stale(self.generation, self.generation_counter, self.cancellation) {
            return Err(TtsError::SynthesisFailed("obsolete native attempt".into()));
        }
        match self.native {
            NativeChoiceExecution::NotRequested => Ok(()),
            NativeChoiceExecution::CommonOnly { reason } => {
                if application.is_none() {
                    *application = Some(degraded(reason)?);
                }
                Ok(())
            }
            NativeChoiceExecution::Parameters(_) => match &*self.receipt.borrow() {
                Receipt::Pending => Err(TtsError::SynthesisFailed(
                    "missing native application receipt".into(),
                )),
                Receipt::Invalid(reason) => Err(TtsError::SynthesisFailed(reason.clone())),
                Receipt::Ready(receipt) => {
                    // Validate every event, but copy bounded evidence only once.
                    if application.is_none() {
                        *application = Some(receipt.clone());
                    }
                    Ok(())
                }
            },
        }
    }
}
