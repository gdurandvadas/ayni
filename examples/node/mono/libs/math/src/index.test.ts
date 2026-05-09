import {describe,it,expect} from 'vitest';
import {add,sub,mul,div,mod,square,cube,max} from './index';
describe('math',()=>{
it('add',()=>expect(add(2,3)).toBe(5));
it('sub',()=>expect(sub(5,2)).toBe(3));
it('mul',()=>expect(mul(2,4)).toBe(8));
it('div',()=>expect(div(8,2)).toBe(4));
it('mod',()=>expect(mod(7,4)).toBe(3));
it('square',()=>expect(square(3)).toBe(9));
it('cube',()=>expect(cube(2)).toBe(8));
it('max',()=>expect(max(3,9)).toBe(9));
});
