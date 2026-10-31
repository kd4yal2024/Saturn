// Offline files only. No socket, radio, PTT or calibration operation.
import {mkdirSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {resolve,join} from 'node:path';
import {createTimingProbePcm} from '../src/puresignal/timing-probe';
const destination = process.argv[2];
if (!destination) throw new Error('Usage: vite-node scripts/generate-ps-timing.mts NEW_DIRECTORY');
const directory=resolve(destination), probe=createTimingProbePcm(1000);
mkdirSync(directory,{mode:0o700}); // never replace an existing evidence directory
const exact=Buffer.alloc(probe.samples.length*4);
probe.samples.forEach((v,i)=>exact.writeFloatLE(v,i*4));
const wav=Buffer.alloc(44+probe.samples.length*2);
wav.write('RIFF',0);wav.writeUInt32LE(wav.length-8,4);wav.write('WAVEfmt ',8);
wav.writeUInt32LE(16,16);wav.writeUInt16LE(1,20);wav.writeUInt16LE(1,22);
wav.writeUInt32LE(48000,24);wav.writeUInt32LE(96000,28);wav.writeUInt16LE(2,32);wav.writeUInt16LE(16,34);
wav.write('data',36);wav.writeUInt32LE(wav.length-44,40);
probe.samples.forEach((v,i)=>wav.writeInt16LE(Math.round(v*32767),44+i*2));
writeFileSync(join(directory,'timing.pcm-f32le'),exact,{flag:'wx',mode:0o600});
writeFileSync(join(directory,'timing-listen-pcm16.wav'),wav,{flag:'wx',mode:0o600});
const {samples,...description}=probe;
writeFileSync(join(directory,'manifest.json'),JSON.stringify({...description,
  format:'saturn-offline-timing-stimulus-v1',samples:samples.length,
  exactPcmSha256:createHash('sha256').update(exact).digest('hex'),
  listeningWavSha256:createHash('sha256').update(wav).digest('hex'),
  physicalRfExecuted:false,fittingAuthorized:false},null,2)+'\n',{flag:'wx',mode:0o600});
console.log(JSON.stringify({directory,offlineOnly:true,samples:samples.length,componentPeak:probe.componentPeak,rms:probe.rms}));
