import { ChangeDetectionStrategy, Component } from '@angular/core';

interface Bar {
  readonly key: 'tga' | 'fbx' | 'png';
  readonly format: string;
  readonly figure: string;
  readonly label: string;
}

@Component({
  selector: 'flr-storage-cards',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './storage-cards.component.html',
  styleUrl: './storage-cards.component.css',
})
export class StorageCardsComponent {
  protected readonly bars: readonly Bar[] = [
    {
      key: 'tga',
      format: '.tga',
      figure: $localize`:@@landing.storage.bar.tga.figure:10.4× smaller`,
      label: $localize`:@@landing.storage.bar.tga.label:A .tga keeps about a tenth of its size`,
    },
    {
      key: 'fbx',
      format: '.fbx',
      figure: $localize`:@@landing.storage.bar.fbx.figure:2.9 to 6.7× smaller`,
      label: $localize`:@@landing.storage.bar.fbx.label:An .fbx keeps between 15 and 34 percent of its size`,
    },
    {
      key: 'png',
      format: '.png',
      figure: $localize`:@@landing.storage.bar.png.figure:1% smaller, stored as it arrived`,
      label: $localize`:@@landing.storage.bar.png.label:A .png keeps about 99 percent of its size`,
    },
  ];
}
