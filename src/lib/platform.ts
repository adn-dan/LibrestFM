/** Version Android (téléphone, tablette) : menu en panneau, scrobbleur. Le PC n'est pas concerné. */
export const android = typeof navigator !== "undefined" && /Android/i.test(navigator.userAgent);
