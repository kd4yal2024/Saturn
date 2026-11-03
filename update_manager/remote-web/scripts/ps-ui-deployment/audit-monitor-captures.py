"""Independent float64 complex FFT over preserved RF pairs; no radio access."""
import cmath,hashlib,json,math,struct,sys
from pathlib import Path
ROOT=Path(sys.argv[1]).resolve() if len(sys.argv)>1 else Path(__file__).resolve().parent

def fft(a):
 n=len(a)
 if n==1:return a
 even,odd=fft(a[::2]),fft(a[1::2]);out=[0j]*n
 for k,(x,y) in enumerate(zip(even,odd)):
  t=cmath.rect(1,-2*math.pi*k/n)*y;out[k]=x+t;out[k+n//2]=x-t
 return out

def analyze(samples,record,compare=True):
 n=len(samples);rate=record['rateHz'];w=[.35875-.48829*math.cos(2*math.pi*i/(n-1))+.14128*math.cos(4*math.pi*i/(n-1))-.01168*math.cos(6*math.pi*i/(n-1)) for i in range(n)]
 sw=sum(w);enbw=n*sum(x*x for x in w)/(sw*sw)
 spectrum=fft([x*y for x,y in zip(samples,w)]);spectrum=spectrum[n//2:]+spectrum[:n//2]
 powers=[abs(x/sw)**2 for x in spectrum]
 def idx(f):return math.floor(f*n/rate+.5)+n//2
 def band(f):return sum(powers[idx(f)-5:idx(f)+6])/enbw
 excluded=record['toneHz']+[p['offsetHz'] for p in record['products']]
 def noise(f):
  bins=[powers[idx(f)+d] for d in range(-80,81) if abs(d)>10 and 0<=idx(f)+d<n and all(abs(idx(f)+d-idx(x))>10 for x in excluded)]
  bins.sort();return bins[len(bins)//2]/math.log(2)*11/enbw
 ref=min(band(f) for f in record['toneHz']);products=[]
 for p in (record['products'] if compare else []):
  power=band(p['offsetHz']);floor=noise(p['offsetHz']);dbc=10*math.log10(max(power-floor,1e-30)/ref)
  products.append({'order':p['order'],'offsetHz':p['offsetHz'],'bridgeDbc':p['dbc'],'float64Dbc':dbc,'differenceDb':None if p['dbc'] is None else dbc-p['dbc'],'productAboveLocalFloorDb':10*math.log10(max(power,1e-30)/max(floor,1e-30))})
 levels=[10*math.log10(band(f)) for f in record['toneHz']]
 pep=20*math.log10(sum(10**(db/20) for db in levels));return {'toneDbfs':levels,'pepMinusWeakerToneDb':pep-min(levels),'binHz':rate/n,'enbwHz':rate/n*enbw,'products':products}

results=[]
for path in sorted((ROOT/'captures').glob('*.json')):
 meta=json.loads(path.read_text());raw=(path.parent/meta['rawFile']).read_bytes();assert hashlib.sha256(raw).hexdigest()==meta['rawSha256'];rows=list(struct.iter_unpack('<ffffQ',raw));assert len(rows)==meta['pairs']
 spectrum=meta['spectrum'];rf=[complex(x[2],x[3]) for x in rows];reference=[complex(x[0],x[1]) for x in rows]
 analysis=analyze(rf,spectrum);results.append({'capture':path.name,'rawSha256Verified':True,'radioFrequencyHz':spectrum['centerHz'],'commandedWatts':spectrum['commandedWatts'],'psRequested':spectrum['psRequested'],'overload':spectrum['overload'],'rf':analysis,'reference':analyze(reference,spectrum,compare=False),'rfPhysicalCalibrationVerified':False})
assert results,'missing captures'
maximum=max(abs(p['differenceDb']) for r in results for p in r['rf']['products'] if p['differenceDb'] is not None)
assert len(results)==2 and all(len(r['rf']['products'])==6 for r in results), 'expected two complete six-product captures'
assert maximum<0.01, f'numerical disagreement: {maximum} dB'
report={'format':'saturn-independent-imd-review-v1','method':'Independent recursive float64 complex FFT; explicit BH4 coherent gain, ENBW and integrated bands; no additive display FFT floor','captures':results,'maximumAbsoluteDifferenceDb':maximum,'reviewKeyedRadio':False,'calibratedRfImdEstablished':False}
(ROOT/'independent-recalculation.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'captures':len(results),'maximumDifferenceDb':maximum,'firstProducts':results[0]['rf']['products'],'pepOffsetDb':results[0]['rf']['pepMinusWeakerToneDb']}))
