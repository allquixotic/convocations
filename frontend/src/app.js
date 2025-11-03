import { h, render } from 'preact';
import { AppRoot } from './app/app-root.js';
import { interceptConsole } from './app/console.js';

interceptConsole();

render(h(AppRoot, null), document.getElementById('app'));
