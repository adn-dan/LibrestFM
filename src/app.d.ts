// Types globaux de SvelteKit.
declare global {
  namespace App {
    interface PageState {
      /** Android : menu en panneau ouvert. */
      menu?: boolean;
      /** Android : panneau de détail ouvert. */
      drill?: boolean;
    }
  }
}

export {};
