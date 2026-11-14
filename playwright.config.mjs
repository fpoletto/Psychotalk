import {defineConfig} from '@playwright/test';
export default defineConfig({testDir:'desktop/tests/browser',workers:1,timeout:120000,use:{headless:true,viewport:{width:1440,height:1000},locale:'pt-BR',permissions:['clipboard-read','clipboard-write']},reporter:'line'});
