// SPDX-License-Identifier: MIT
// Omnivox's checked overlay for the separately licensed libpiper build copy.
// Included after piper_impl.hpp and eSpeak's declarations in piper.cpp.
#include <atomic>
#include <cstdio>
#include <exception>
#include <memory>

namespace {
// eSpeak's native state is process-global. A helper may own one Piper model;
// reject overlapping construction rather than terminating another model's
// phonemizer when a failed constructor unwinds.
std::atomic<bool> omnivox_piper_model_owned{false};

struct OmnivoxPiperConstruction {
  bool owns_slot = !omnivox_piper_model_owned.exchange(true);
  bool phonemizer_attempted = false;
  std::unique_ptr<piper_synthesizer> synth;

  ~OmnivoxPiperConstruction() {
    if (!owns_slot) {
      return;
    }
    if (phonemizer_attempted) {
      espeak_Terminate();
    }
    synth.reset();
    omnivox_piper_model_owned.store(false);
  }

  piper_synthesizer *release() {
    owns_slot = false;
    return synth.release();
  }
};

void omnivox_piper_exception(const char *operation, const char *message) noexcept {
  // Bound diagnostics and keep them off the framed stdout protocol.
  std::fprintf(stderr, "Piper %s failed: %.1024s\n", operation, message);
}
} // namespace
