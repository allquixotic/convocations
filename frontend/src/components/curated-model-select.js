import { h } from 'preact';

export function CuratedModelSelect({
  curatedModels,
  value,
  onChange,
  fieldClass,
  autoValue,
  manualValue,
}) {
  if (!Array.isArray(curatedModels) || curatedModels.length === 0) {
    return null;
  }

  const formatLabel = (model) => {
    const priceNote =
      typeof model.price_in_per_million === 'number'
        ? ` · $${model.price_in_per_million.toFixed(2)}/1M`
        : '';
    const tierLabel = model.tier === 'free' ? 'Free' : 'Cheap';
    return `${tierLabel} · ${model.display_name} (${model.slug}) · AAII ${model.aaii.toFixed(
      1,
    )}${priceNote}`;
  };

  return h(
    'div',
    { class: 'field-grid', style: 'margin-top: 1rem;' },
    h(
      'label',
      { class: fieldClass },
      h('span', { class: 'field-label' }, 'Curated Models'),
      h(
        'select',
        {
          value,
          onChange,
          style: 'width: 100%;',
        },
        h('option', { value: autoValue }, 'Automatic (recommended)'),
        curatedModels.map((model) => {
          const label = formatLabel(model);
          return h(
            'option',
            {
              key: model.slug,
              value: model.slug,
              title: label,
            },
            label,
          );
        }),
        h('option', { value: manualValue }, 'Manual selection'),
      ),
      h(
        'span',
        { class: 'field-hint' },
        'Choose a curated model or switch to manual entry below.',
      ),
    ),
  );
}
