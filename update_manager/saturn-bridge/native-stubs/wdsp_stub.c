// CI-only WDSP native stub.
//
// Production builds link the real piHPSDR WDSP/RNNoise/SpecBleach static
// libraries. GitHub runners do not have that tree, so build.rs can compile this
// stub when SATURN_BRIDGE_STUB_NATIVE=1 to let Rust parser/control tests run.

#include <stdint.h>

void OpenChannel(int32_t channel, int32_t in_size, int32_t dsp_size,
                 int32_t input_samplerate, int32_t dsp_rate,
                 int32_t output_samplerate, int32_t channel_type,
                 int32_t state, double tdelayup, double tslewup,
                 double tdelaydown, double tslewdown, int32_t bfo) {
  (void)channel; (void)in_size; (void)dsp_size; (void)input_samplerate;
  (void)dsp_rate; (void)output_samplerate; (void)channel_type; (void)state;
  (void)tdelayup; (void)tslewup; (void)tdelaydown; (void)tslewdown; (void)bfo;
}

void CloseChannel(int32_t channel) { (void)channel; }
int32_t SetChannelState(int32_t channel, int32_t state, int32_t dmode) {
  (void)channel; (void)state; (void)dmode; return 0;
}
void fexchange0(int32_t channel, const double *input, double *output,
                int32_t *error) {
  (void)channel; (void)input; (void)output;
  if (error) *error = 0;
}

#define STUB_VOID_I32_I32(name) void name(int32_t a, int32_t b) { (void)a; (void)b; }
#define STUB_VOID_I32_F64(name) void name(int32_t a, double b) { (void)a; (void)b; }
#define STUB_VOID_I32_F32(name) void name(int32_t a, float b) { (void)a; (void)b; }
#define STUB_VOID_I32_I32_I32(name) void name(int32_t a, int32_t b, int32_t c) { (void)a; (void)b; (void)c; }
#define STUB_VOID_I32_F64_F64(name) void name(int32_t a, double b, double c) { (void)a; (void)b; (void)c; }

STUB_VOID_I32_I32(SetRXAMode)
STUB_VOID_I32_F64_F64(RXASetPassband)
STUB_VOID_I32_I32(RXASetNC)
STUB_VOID_I32_I32(RXASetMP)
STUB_VOID_I32_I32(SetRXABandpassWindow)
STUB_VOID_I32_I32(SetRXABandpassRun)
STUB_VOID_I32_I32(SetRXAAMDSBMode)
STUB_VOID_I32_I32(SetRXAPanelRun)
STUB_VOID_I32_I32(SetRXAPanelSelect)
STUB_VOID_I32_I32(SetRXAPanelCopy)
STUB_VOID_I32_F64(SetRXAPanelGain1)

void create_anbEXT(int32_t id, int32_t run, int32_t buffsize, double samplerate,
                   double tau, double hangtime, double advtime, double backtau,
                   double threshold) {
  (void)id; (void)run; (void)buffsize; (void)samplerate; (void)tau;
  (void)hangtime; (void)advtime; (void)backtau; (void)threshold;
}
void destroy_anbEXT(int32_t id) { (void)id; }
STUB_VOID_I32_I32(SetEXTANBRun)
STUB_VOID_I32_F64(SetEXTANBTau)
STUB_VOID_I32_F64(SetEXTANBHangtime)
STUB_VOID_I32_F64(SetEXTANBAdvtime)
STUB_VOID_I32_F64(SetEXTANBThreshold)

void create_nobEXT(int32_t id, int32_t run, int32_t mode, int32_t buffsize,
                   double samplerate, double slewtime, double hangtime,
                   double advtime, double backtau, double threshold) {
  (void)id; (void)run; (void)mode; (void)buffsize; (void)samplerate;
  (void)slewtime; (void)hangtime; (void)advtime; (void)backtau;
  (void)threshold;
}
void destroy_nobEXT(int32_t id) { (void)id; }
STUB_VOID_I32_I32(SetEXTNOBRun)
STUB_VOID_I32_I32(SetEXTNOBMode)
STUB_VOID_I32_F64(SetEXTNOBTau)
STUB_VOID_I32_F64(SetEXTNOBHangtime)
STUB_VOID_I32_F64(SetEXTNOBAdvtime)
STUB_VOID_I32_F64(SetEXTNOBThreshold)

STUB_VOID_I32_I32(SetRXAANFRun)
void SetRXAANFVals(int32_t channel, int32_t taps, int32_t delay, double gain,
                   double leakage) {
  (void)channel; (void)taps; (void)delay; (void)gain; (void)leakage;
}
STUB_VOID_I32_I32(SetRXAANFPosition)
STUB_VOID_I32_I32(SetRXAANRRun)
void SetRXAANRVals(int32_t channel, int32_t taps, int32_t delay, double gain,
                   double leakage) {
  (void)channel; (void)taps; (void)delay; (void)gain; (void)leakage;
}
STUB_VOID_I32_I32(SetRXAANRPosition)
STUB_VOID_I32_I32(SetRXAEMNRRun)
STUB_VOID_I32_I32(SetRXAEMNRgainMethod)
STUB_VOID_I32_I32(SetRXAEMNRnpeMethod)
STUB_VOID_I32_I32(SetRXAEMNRaeRun)
STUB_VOID_I32_I32(SetRXAEMNRPosition)
STUB_VOID_I32_F64(SetRXAEMNRtrainZetaThresh)
STUB_VOID_I32_F64(SetRXAEMNRtrainT2)
STUB_VOID_I32_I32(SetRXAEMNRpost2Run)
STUB_VOID_I32_F64(SetRXAEMNRpost2Nlevel)
STUB_VOID_I32_F64(SetRXAEMNRpost2Factor)
STUB_VOID_I32_F64(SetRXAEMNRpost2Rate)
STUB_VOID_I32_I32(SetRXAEMNRpost2Taper)
STUB_VOID_I32_I32(SetRXARNNRRun)
void RNNRloadModel(const char *file_path) { (void)file_path; }
STUB_VOID_I32_I32(SetRXARNNRPosition)
STUB_VOID_I32_I32(SetRXASBNRRun)
STUB_VOID_I32_F32(SetRXASBNRreductionAmount)
STUB_VOID_I32_F32(SetRXASBNRsmoothingFactor)
STUB_VOID_I32_F32(SetRXASBNRwhiteningFactor)
STUB_VOID_I32_F32(SetRXASBNRnoiseRescale)
STUB_VOID_I32_F32(SetRXASBNRpostFilterThreshold)
STUB_VOID_I32_I32(SetRXASBNRnoiseScalingType)
STUB_VOID_I32_I32(SetRXASBNRPosition)

STUB_VOID_I32_I32(SetRXAAGCMode)
STUB_VOID_I32_I32(SetRXAAGCSlope)
STUB_VOID_I32_F64(SetRXAAGCTop)
STUB_VOID_I32_I32(SetRXAAGCAttack)
STUB_VOID_I32_I32(SetRXAAGCHang)
STUB_VOID_I32_I32(SetRXAAGCDecay)
STUB_VOID_I32_I32(SetRXAAGCHangThreshold)
STUB_VOID_I32_F64(SetRXAAGCMaxInputLevel)
double GetRXAMeter(int32_t channel, int32_t mt) { (void)channel; (void)mt; return 0.0; }

STUB_VOID_I32_I32(SetTXAMode)
STUB_VOID_I32_I32(SetTXABandpassWindow)
STUB_VOID_I32_I32(SetTXABandpassRun)
STUB_VOID_I32_F64_F64(SetTXABandpassFreqs)
STUB_VOID_I32_I32(SetTXACFIRRun)
STUB_VOID_I32_I32(SetTXAAMSQRun)
STUB_VOID_I32_F64(SetTXAAMSQThreshold)
STUB_VOID_I32_F64(SetTXAAMSQMutedGain)
STUB_VOID_I32_I32(SetTXAALCSt)
STUB_VOID_I32_I32(SetTXAALCAttack)
STUB_VOID_I32_I32(SetTXAALCDecay)
STUB_VOID_I32_F64(SetTXAALCMaxGain)
STUB_VOID_I32_I32(SetTXALevelerSt)
STUB_VOID_I32_I32(SetTXALevelerAttack)
STUB_VOID_I32_I32(SetTXALevelerDecay)
STUB_VOID_I32_F64(SetTXALevelerTop)
STUB_VOID_I32_I32(SetTXAPreGenMode)
STUB_VOID_I32_F64(SetTXAPreGenToneMag)
STUB_VOID_I32_F64(SetTXAPreGenToneFreq)
STUB_VOID_I32_I32(SetTXAPreGenRun)
STUB_VOID_I32_I32(SetTXAPanelRun)
STUB_VOID_I32_I32(SetTXAPanelSelect)
STUB_VOID_I32_F64(SetTXAPanelGain1)
STUB_VOID_I32_I32(SetTXAPostGenRun)

STUB_VOID_I32_I32(SetRXAEQRun)
void SetRXAGrphEQ10(int32_t channel, int32_t *rxeq) { (void)channel; (void)rxeq; }
STUB_VOID_I32_I32(SetTXAEQRun)
void SetTXAGrphEQ10(int32_t channel, int32_t *txeq) { (void)channel; (void)txeq; }

STUB_VOID_I32_I32(SetTXACFCOMPRun)
STUB_VOID_I32_I32(SetTXACFCOMPPosition)
void SetTXACFCOMPprofile(int32_t channel, int32_t nfreqs, const double *f,
                         const double *g, const double *e) {
  (void)channel; (void)nfreqs; (void)f; (void)g; (void)e;
}
STUB_VOID_I32_F64(SetTXACFCOMPPrecomp)
STUB_VOID_I32_I32(SetTXACFCOMPPeqRun)
STUB_VOID_I32_F64(SetTXACFCOMPPrePeq)

STUB_VOID_I32_I32(SetTXAPostGenMode)
void SetTXAPostGenTTMag(int32_t channel, double mag1, double mag2) {
  (void)channel; (void)mag1; (void)mag2;
}
void SetTXAPostGenTTFreq(int32_t channel, double freq1, double freq2) {
  (void)channel; (void)freq1; (void)freq2;
}
STUB_VOID_I32_I32(TXASetNC)
STUB_VOID_I32_I32(TXASetMP)
double GetTXAMeter(int32_t channel, int32_t mt) { (void)channel; (void)mt; return 0.0; }

STUB_VOID_I32_I32(SetDEXPRun)
STUB_VOID_I32_F64(SetDEXPDetectorTau)
STUB_VOID_I32_F64(SetDEXPAttackTime)
STUB_VOID_I32_F64(SetDEXPReleaseTime)
STUB_VOID_I32_F64(SetDEXPHoldTime)
STUB_VOID_I32_F64(SetDEXPExpansionRatio)
STUB_VOID_I32_F64(SetDEXPHysteresisRatio)
STUB_VOID_I32_F64(SetDEXPAttackThreshold)
STUB_VOID_I32_F64(SetDEXPLowCut)
STUB_VOID_I32_F64(SetDEXPHighCut)
STUB_VOID_I32_I32(SetDEXPRunSideChannelFilter)
