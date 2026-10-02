import { Component, input } from '@angular/core';

@Component({
  selector: 'lfsx-logo',
  templateUrl: './logo.html',
  styleUrl: './logo.css',
})
export class Logo {
  readonly wordmark = input(true);
}
