/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        momo: {
          50:  "#fdf2ff",
          100: "#fae5ff",
          200: "#f3ccff",
          300: "#e9a3ff",
          400: "#d766fc",
          500: "#c040f3",
          600: "#a31fd6",
          700: "#8717af",
          800: "#6e168e",
          900: "#5a1672",
        },
      },
    },
  },
  plugins: [],
};
