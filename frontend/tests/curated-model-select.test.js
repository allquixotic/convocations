import { describe, expect, it } from 'bun:test';
import { h } from 'preact';
import renderToString from 'preact-render-to-string';
import { CuratedModelSelect } from '../src/components/curated-model-select.js';

const AUTO = 'auto';
const MANUAL = 'manual';

describe('CuratedModelSelect', () => {
  const baseProps = {
    onChange: () => {},
    fieldClass: 'field field--full',
    autoValue: AUTO,
    manualValue: MANUAL,
  };

  it('renders curated options along with automatic and manual choices', () => {
    const curatedModels = [
      {
        slug: 'provider/pro-free',
        display_name: 'Free Model',
        tier: 'free',
        aaii: 71.2,
        price_in_per_million: 0,
      },
      {
        slug: 'provider/pro-cheap',
        display_name: 'Budget Model',
        tier: 'cheap',
        aaii: 67.4,
        price_in_per_million: 0.85,
      },
    ];

    const markup = renderToString(
      h(CuratedModelSelect, {
        ...baseProps,
        curatedModels,
        value: AUTO,
      }),
    );

    expect(markup).toContain('Curated Models');
    expect(markup).toContain('Automatic (recommended)');
    expect(markup).toContain('value="provider/pro-free"');
    expect(markup).toContain('Free Model');
    expect(markup).toContain('value="provider/pro-cheap"');
    expect(markup).toContain('$0.85/1M');
    expect(markup).toContain('Manual selection');
  });

  it('renders nothing when no curated models are provided', () => {
    const markup = renderToString(
      h(CuratedModelSelect, {
        ...baseProps,
        curatedModels: [],
        value: AUTO,
      }),
    );

    expect(markup).toBe('');
  });
});
