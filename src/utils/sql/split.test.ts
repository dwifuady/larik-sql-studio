import { describe, it, expect } from 'vitest';
import { splitSqlStatements } from './split';

describe('splitSqlStatements', () => {
  it('single statement', () => {
    expect(splitSqlStatements('SELECT 1')).toEqual(['SELECT 1']);
  });
  it('two statements', () => {
    expect(splitSqlStatements('SELECT 1; SELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('trailing semicolon', () => {
    expect(splitSqlStatements('SELECT 1;')).toEqual(['SELECT 1']);
  });
  it('empty', () => {
    expect(splitSqlStatements('')).toEqual([]);
  });
  it('only semicolons', () => {
    expect(splitSqlStatements(';;;')).toEqual([]);
  });
  it('string with semicolon', () => {
    expect(splitSqlStatements("SELECT 'a;b'")).toEqual(["SELECT 'a;b'"]);
  });
  it('escaped string semicolon', () => {
    expect(splitSqlStatements("SELECT 'a'';b'")).toEqual(["SELECT 'a'';b'"]);
  });
  it('bracket with semicolon', () => {
    expect(splitSqlStatements('SELECT [my;col] FROM t')).toEqual(['SELECT [my;col] FROM t']);
  });
  it('bracket escaped', () => {
    expect(splitSqlStatements('SELECT [my]]col] FROM t')).toEqual(['SELECT [my]]col] FROM t']);
  });
  it('line comment with semicolon', () => {
    expect(splitSqlStatements('SELECT 1 -- comment ;\nSELECT 2')).toEqual(['SELECT 1 -- comment ;\nSELECT 2']);
  });
  it('block comment with semicolon', () => {
    expect(splitSqlStatements('SELECT 1 /* ; */; SELECT 2')).toEqual(['SELECT 1 /* ; */', 'SELECT 2']);
  });
  it('block comment multiline', () => {
    expect(splitSqlStatements('SELECT 1 /*\n ; \n */; SELECT 2')).toEqual(['SELECT 1 /*\n ; \n */', 'SELECT 2']);
  });
  it('GO separator', () => {
    expect(splitSqlStatements('SELECT 1\nGO\nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('GO with count', () => {
    expect(splitSqlStatements('SELECT 1\nGO 10\nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('GO case insensitive', () => {
    expect(splitSqlStatements('select 1\ngo\nselect 2')).toEqual(['select 1', 'select 2']);
  });
  it('GO with trailing spaces', () => {
    expect(splitSqlStatements('SELECT 1\nGO   \nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('GO with comment', () => {
    expect(splitSqlStatements('SELECT 1\nGO -- comment\nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('GO inside string not split', () => {
    expect(splitSqlStatements("SELECT 'GO'")).toEqual(["SELECT 'GO'"]);
  });
  it('GO inside block comment not split', () => {
    expect(splitSqlStatements('SELECT 1 /* GO */')).toEqual(['SELECT 1 /* GO */']);
  });
  it('multiple GO', () => {
    expect(splitSqlStatements('SELECT 1\nGO\nGO\nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('GO at start', () => {
    expect(splitSqlStatements('GO\nSELECT 1')).toEqual(['SELECT 1']);
  });
  it('semicolon and GO mixed', () => {
    expect(splitSqlStatements('SELECT 1;\nGO\nSELECT 2;')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('bracket GO', () => {
    expect(splitSqlStatements('SELECT [GO]')).toEqual(['SELECT [GO]']);
  });
  it('string with bracket and semicolon', () => {
    expect(splitSqlStatements("SELECT 'a[;b'")).toEqual(["SELECT 'a[;b'"]);
  });
  it('empty statements trimmed', () => {
    expect(splitSqlStatements('  SELECT 1  ;  ; SELECT 2  ')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('newline handling', () => {
    expect(splitSqlStatements('SELECT 1\n;\nSELECT 2')).toEqual(['SELECT 1', 'SELECT 2']);
  });
  it('string with newline and semicolon', () => {
    expect(splitSqlStatements("SELECT 'a\n;b'")).toEqual(["SELECT 'a\n;b'"]);
  });
  it('block comment with GO and semicolon', () => {
    expect(splitSqlStatements('SELECT 1 /* GO; */; SELECT 2')).toEqual(['SELECT 1 /* GO; */', 'SELECT 2']);
  });
  it('line comment at end', () => {
    expect(splitSqlStatements('SELECT 1 -- comment')).toEqual(['SELECT 1 -- comment']);
  });
  it('complex', () => {
    expect(
      splitSqlStatements("SELECT [a]; -- comment\nGO\nSELECT 'b;c' /* ; */")
    ).toEqual(["SELECT [a]", "-- comment", "SELECT 'b;c' /* ; */"]);
  });
});
