import assert from 'node:assert/strict';
import { test } from 'node:test';
import { countryNameIn, placeText, shortPlace } from './geo.ts';

const fake = (code: string): string | null => (code === 'XA' ? 'Examplia' : null);

test('placeText joins city, region and country; missing parts are left out', () => {
  assert.equal(placeText({ country: 'XA', region: 'North', city: 'Town' }, fake), 'Town, North, Examplia');
  assert.equal(placeText({ country: 'XA', region: null, city: 'Town' }, fake), 'Town, Examplia');
  assert.equal(placeText({ country: 'XA', region: null, city: null }, fake), 'Examplia');
});

test('placeText falls back to the code and never invents a place', () => {
  assert.equal(placeText({ country: 'ZZ', region: null, city: null }, fake), 'ZZ');
  assert.equal(placeText(null, fake), null);
  assert.equal(placeText(undefined, fake), null);
  assert.equal(placeText({ country: '', region: ' ', city: null }, fake), null);
});

test('countryNameIn never throws on a bad code', () => {
  const n = countryNameIn('en');
  assert.equal(typeof n('US') === 'string' || n('US') === null, true);
  assert.equal(n('not-a-code'), null);
});

test('placeText accepts a missing country (an API provider may give none)', () => {
  assert.equal(placeText({ country: null, region: 'North', city: 'Example City' }, fake), 'Example City, North');
  assert.equal(placeText({ country: null, region: null, city: null }, fake), null);
});

test('shortPlace: city, else region, else country name; raw country text kept', () => {
  assert.equal(shortPlace({ country: 'XA', region: 'North', city: 'Example City' }, fake), 'Example City');
  assert.equal(shortPlace({ country: 'XA', region: 'North', city: null }, fake), 'North');
  assert.equal(shortPlace({ country: 'XA', region: null, city: ' ' }, fake), 'Examplia');
  assert.equal(shortPlace({ country: 'ZZ', region: null, city: null }, fake), 'ZZ');
  assert.equal(shortPlace({ country: 'Example Land', region: null, city: null }, fake), 'Example Land');
  assert.equal(shortPlace({ country: null, region: null, city: null }, fake), null);
  assert.equal(shortPlace(null, fake), null);
});
