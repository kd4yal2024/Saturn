# Saturn Remote Web

This directory is the staged TypeScript replacement for the monolithic
`templates/saturn-remote.html` browser code.

The first goal is extraction without behavior change:

- keep the deployed `saturn-remote.html` working while modules are built here
- move pure parsing/formatting/state helpers first
- add tests before moving PTT, audio, and WebGL behavior
- later add a Vite build that emits the deployed browser bundle

## Initial Modules

- `src/tci/parser.ts` parses semicolon-separated TCI text messages.
- `src/radio/frequency.ts` formats and steps VFO/DDS frequencies.
- `src/radio/passband.ts` converts between UI filter cuts and signed TCI
  passbands.
- `src/transport/tci-frame.ts` parses the 64-byte binary TCI-style frame
  header used for IQ/audio frames.

## Commands

Once Node dependencies are installed:

```bash
npm install
npm test
npm run typecheck
```

This scaffold intentionally does not change the current Saturn Go template
copy/deploy path yet.
