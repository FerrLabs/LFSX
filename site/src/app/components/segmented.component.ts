import { ChangeDetectionStrategy, Component, input, model } from '@angular/core';

export interface SegmentOption<K extends string> {
  readonly key: K;
  readonly label: string;
}

@Component({
  selector: 'flr-segmented',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './segmented.component.html',
  styleUrl: './segmented.component.css',
})
export class SegmentedComponent<K extends string> {
  readonly label = input.required<string>();
  readonly options = input.required<readonly SegmentOption<K>[]>();
  readonly value = model.required<K>();
}
