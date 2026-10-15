/* Milestone 1 proposal only; not included by production code.
 * Contract pinned to TAPR WDSP b02d5bac675dd2f33ec2bab2b339f79a597c47dd.
 * Compile this AFTER that revision's wdsp.h: incompatible declarations fail.
 * A source/header check is not proof of an installed library's ABI.
 */
#ifndef SATURN_PS_M1_NATIVE_ABI_H
#define SATURN_PS_M1_NATIVE_ABI_H

enum {
    SATURN_PS_INFO_INTS = 16,
    SATURN_PS_INPUT_PAIRS = 1024,
    SATURN_PS_INPUT_DOUBLES = 2048,
    SATURN_PS_MEASURED_CAPACITY = 4096,
    SATURN_PS_CORRECTION_CAPACITY = 512
};

typedef struct {
    double x[SATURN_PS_MEASURED_CAPACITY];
    double ym[SATURN_PS_MEASURED_CAPACITY];
    double yc[SATURN_PS_MEASURED_CAPACITY];
    double ys[SATURN_PS_MEASURED_CAPACITY];
    double xm_cor[SATURN_PS_CORRECTION_CAPACITY];
    double ym_cor[SATURN_PS_CORRECTION_CAPACITY];
    double xa_cor[SATURN_PS_CORRECTION_CAPACITY];
    double ya_cor[SATURN_PS_CORRECTION_CAPACITY];
    int nsamps_out;
    int cpts_out;
    double phs_ref_deg_out;
} SaturnPsDisplayStorage;

void pscc(int channel, int size, double *tx, double *rx);
void GetPSInfo(int channel, int *info);
void GetPSDisp(int channel, double *x, double *ym, double *yc, double *ys,
               double *xm_cor, double *ym_cor, double *xa_cor, double *ya_cor,
               int *nsamps_out, int *cpts_out, double *phs_ref_deg_out);
void GetPSMaxTX(int channel, double *maxtx);
void GetPSHWPeak(int channel, double *peak);
void SetPSMox(int channel, int mox);
void SetPSControl(int channel, int reset, int mancal, int automode, int turnon);
void SetPSFeedbackRate(int channel, int rate);
void SetPSHWPeak(int channel, double peak);
void SetPSMoxDelay(int channel, double delay_seconds);
void SetPSLoopDelay(int channel, double delay_seconds);
double SetPSTXDelay(int channel, double delay_seconds);
int GetWDSPVersion(void);
void OpenChannel(int channel, int input_size, int dsp_size, int input_rate,
                 int dsp_rate, int output_rate, int type, int state,
                 double delay_up, double slew_up, double delay_down,
                 double slew_down, int bfo);
int SetChannelState(int channel, int state, int wait);
void CloseChannel(int channel);
void fexchange0(int channel, double *in, double *out, int *error);

_Static_assert(sizeof(int) == 4, "WDSP int must match Rust i32");
_Static_assert(sizeof(double) == 8, "WDSP double must match Rust f64");
_Static_assert(SATURN_PS_INPUT_DOUBLES == 2 * SATURN_PS_INPUT_PAIRS,
               "pscc size is complex pairs, not double elements");
_Static_assert(_Generic(&GetPSDisp,
    void (*)(int, double *, double *, double *, double *,
             double *, double *, double *, double *, int *, int *, double *): 1,
    default: 0), "GetPSDisp ABI mismatch");
_Static_assert(_Generic(&pscc,
    void (*)(int, int, double *, double *): 1, default: 0), "pscc ABI mismatch");
_Static_assert(_Generic(&SetPSTXDelay,
    double (*)(int, double): 1, default: 0), "SetPSTXDelay ABI mismatch");
_Static_assert(_Generic(&OpenChannel,
    void (*)(int, int, int, int, int, int, int, int, double, double, double, double, int): 1,
    default: 0), "OpenChannel ABI mismatch");
_Static_assert(_Generic(&fexchange0,
    void (*)(int, double *, double *, int *): 1, default: 0), "fexchange0 ABI mismatch");

#endif
