declare module '*.css?inline' {
  const css: string;
  export default css;
}

declare module '*.svg?raw' {
  const svg: string;
  export default svg;
}

declare module '*.txt?raw' {
  const text: string;
  export default text;
}
