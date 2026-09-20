// Static scope analysis for the inline <script> blocks in update_manager/templates/*.html.
//
// The templates are hand-maintained HTML with large inline scripts that nothing ever
// parses before deployment. A `const` declared inside one `if` block and used from a
// sibling block is a ReferenceError that only fires on the live radio, and the
// string-matching template tests cannot see it. This walks the real AST instead.

import * as acorn from 'acorn';

/** Inline (non-`src`) script bodies, with their byte offset into the original HTML. */
export function inlineScripts(html) {
  const scripts = [];
  const pattern = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi;
  let match;
  while ((match = pattern.exec(html))) {
    const attrs = match[1];
    if (/\bsrc\s*=/i.test(attrs)) continue;
    if (/\btype\s*=\s*["'](?!(?:module|text\/javascript|application\/javascript)\b)/i.test(attrs)) continue;
    scripts.push({ code: match[2], offset: match.index + match[0].indexOf(match[2]) });
  }
  return scripts;
}

const FUNCTION_SCOPE = 'function';
const BLOCK_SCOPE = 'block';

/**
 * Identifier references that resolve to no binding in any enclosing scope and are not
 * in `globals`. Returns `{ name, start }` where `start` is an offset into `code`.
 */
export function findUnresolvedIdentifiers(code, { globals = [] } = {}) {
  const ast = acorn.parse(code, { ecmaVersion: 2022, sourceType: 'script' });
  const globalNames = new Set(globals);
  const unresolved = [];

  const createScope = (parent, type) => ({ parent, type, names: new Set() });

  const declare = (pattern, scope) => {
    if (!pattern) return;
    switch (pattern.type) {
      case 'Identifier':
        scope.names.add(pattern.name);
        break;
      case 'ObjectPattern':
        for (const property of pattern.properties) {
          declare(property.type === 'RestElement' ? property.argument : property.value, scope);
        }
        break;
      case 'ArrayPattern':
        for (const element of pattern.elements) declare(element, scope);
        break;
      case 'AssignmentPattern':
        declare(pattern.left, scope);
        break;
      case 'RestElement':
        declare(pattern.argument, scope);
        break;
      default:
        break;
    }
  };

  // `var` and function declarations hoist to the enclosing function scope; don't
  // descend into nested functions, which start a scope of their own.
  const hoistVarBindings = (node, functionScope) => {
    if (!node || typeof node.type !== 'string') return;
    if (
      node.type === 'FunctionDeclaration' ||
      node.type === 'FunctionExpression' ||
      node.type === 'ArrowFunctionExpression' ||
      node.type === 'ClassDeclaration' ||
      node.type === 'ClassExpression'
    ) {
      return;
    }
    if (node.type === 'VariableDeclaration' && node.kind === 'var') {
      for (const declarator of node.declarations) declare(declarator.id, functionScope);
    }
    for (const key of Object.keys(node)) {
      if (key === 'type') continue;
      const value = node[key];
      if (Array.isArray(value)) {
        for (const child of value) if (child && typeof child.type === 'string') hoistVarBindings(child, functionScope);
      } else if (value && typeof value.type === 'string') {
        hoistVarBindings(value, functionScope);
      }
    }
  };

  // `let`/`const`/`function`/`class` introduced directly by a statement list.
  const hoistLexicalBindings = (body, scope) => {
    for (const statement of body) {
      if (!statement) continue;
      if (statement.type === 'VariableDeclaration' && statement.kind !== 'var') {
        for (const declarator of statement.declarations) declare(declarator.id, scope);
      } else if ((statement.type === 'FunctionDeclaration' || statement.type === 'ClassDeclaration') && statement.id) {
        scope.names.add(statement.id.name);
      }
    }
  };

  const resolves = (name, scope) => {
    for (let current = scope; current; current = current.parent) {
      if (current.names.has(name)) return true;
    }
    return globalNames.has(name);
  };

  const walkFunction = (node, scope) => {
    const functionScope = createScope(scope, FUNCTION_SCOPE);
    functionScope.names.add('arguments');
    if (node.id && node.type !== 'FunctionDeclaration') functionScope.names.add(node.id.name);
    for (const param of node.params) declare(param, functionScope);
    if (node.body.type === 'BlockStatement') {
      for (const statement of node.body.body) hoistVarBindings(statement, functionScope);
      hoistLexicalBindings(node.body.body, functionScope);
      for (const statement of node.body.body) walk(statement, functionScope);
    } else {
      walk(node.body, functionScope);
    }
  };

  const walkChildren = (node, scope) => {
    for (const key of Object.keys(node)) {
      if (key === 'type' || key === 'start' || key === 'end') continue;
      const value = node[key];
      if (Array.isArray(value)) {
        for (const child of value) if (child && typeof child.type === 'string') walk(child, scope);
      } else if (value && typeof value.type === 'string') {
        walk(value, scope);
      }
    }
  };

  function walk(node, scope) {
    if (!node || typeof node.type !== 'string') return;
    switch (node.type) {
      case 'Program': {
        const programScope = createScope(null, FUNCTION_SCOPE);
        for (const statement of node.body) hoistVarBindings(statement, programScope);
        hoistLexicalBindings(node.body, programScope);
        for (const statement of node.body) walk(statement, programScope);
        return;
      }
      case 'Identifier':
        if (!resolves(node.name, scope)) unresolved.push({ name: node.name, start: node.start });
        return;
      case 'FunctionDeclaration':
      case 'FunctionExpression':
      case 'ArrowFunctionExpression':
        walkFunction(node, scope);
        return;
      case 'BlockStatement': {
        const blockScope = createScope(scope, BLOCK_SCOPE);
        hoistLexicalBindings(node.body, blockScope);
        for (const statement of node.body) walk(statement, blockScope);
        return;
      }
      case 'ForStatement':
      case 'ForInStatement':
      case 'ForOfStatement': {
        const loopScope = createScope(scope, BLOCK_SCOPE);
        for (const key of ['init', 'left']) {
          const clause = node[key];
          if (clause && clause.type === 'VariableDeclaration' && clause.kind !== 'var') {
            for (const declarator of clause.declarations) declare(declarator.id, loopScope);
          }
        }
        walkChildren(node, loopScope);
        return;
      }
      case 'CatchClause': {
        const catchScope = createScope(scope, BLOCK_SCOPE);
        if (node.param) declare(node.param, catchScope);
        hoistLexicalBindings(node.body.body, catchScope);
        for (const statement of node.body.body) walk(statement, catchScope);
        return;
      }
      case 'ClassDeclaration':
      case 'ClassExpression': {
        const classScope = createScope(scope, BLOCK_SCOPE);
        if (node.id) classScope.names.add(node.id.name);
        if (node.superClass) walk(node.superClass, classScope);
        walk(node.body, classScope);
        return;
      }
      case 'ClassBody':
        for (const element of node.body) walk(element, scope);
        return;
      case 'MemberExpression':
        walk(node.object, scope);
        if (node.computed) walk(node.property, scope);
        return;
      case 'Property':
      case 'PropertyDefinition':
      case 'MethodDefinition':
        if (node.computed) walk(node.key, scope);
        if (node.value) walk(node.value, scope);
        return;
      case 'VariableDeclaration':
        // Bindings are already hoisted by the enclosing scope; only initialisers reference.
        for (const declarator of node.declarations) if (declarator.init) walk(declarator.init, scope);
        return;
      case 'LabeledStatement':
        walk(node.body, scope);
        return;
      case 'BreakStatement':
      case 'ContinueStatement':
        return;
      default:
        walkChildren(node, scope);
    }
  }

  walk(ast, null);
  return unresolved;
}

export const lineOf = (text, offset) => text.slice(0, offset).split('\n').length;
